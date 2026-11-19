//! 重连视图模型（FR-05.4，对齐 `docs/architecture.md` §4.2「QUIC 断连」行 + M7 reconnect）。
//!
//! 把 [`landesk_core::ReconnectScheduler`] 的纯调度结果翻译成语义化 UI 提示
//! （状态栏 / 重连倒计时文案），喂 `ui/frontend` 的重连指示器。纯逻辑可离线单测。

use landesk_core::ReconnectScheduler;

/// 一次重连尝试的 UI 视图（含剩余重试机会，供前端显示「第 N/M 次，Xs 后重试」）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReconnectView {
    /// 当前是第几次尝试（1 起）。
    pub attempt: u32,
    /// 最多重试次数（默认 5）。
    pub max_attempts: u32,
    /// 本次前应等待的退避秒数（1/2/4/8/16）。
    pub wait_secs: u64,
    /// 本次视图是否已无下次（恒 `false`——已耗尽时 [`view_of`] 直接返 `None`）。
    pub exhausted: bool,
    /// 展示文案（中文，喂状态栏）。
    pub label: String,
}

/// 由调度器当前进度构造**下一次**重试的 UI 视图（纯查询，不推进入参）。
///
/// 已耗尽（`used >= max`）→ `None`（前端转 [`landesk_core::SessionState::NoPath`]）。
pub fn view_of(sched: &ReconnectScheduler) -> Option<ReconnectView> {
    if sched.exhausted() {
        return None;
    }
    let attempt = sched.used() + 1;
    let max = sched.max_attempts();
    let wait = sched.policy().delay_for(attempt - 1).unwrap_or_default();
    Some(ReconnectView {
        attempt,
        max_attempts: max,
        wait_secs: wait.as_secs(),
        exhausted: false,
        label: format!("重连中（第 {attempt}/{max} 次，{}s 后重试）", wait.as_secs()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use landesk_core::BackoffPolicy;

    #[test]
    fn view_counts_up_with_backoff() {
        let mut s = ReconnectScheduler::default_policy();
        s.step(); // 第 1 次，已用 1
        let v = view_of(&s).expect("not exhausted yet");
        assert_eq!(v.attempt, 2, "已用 1 次后，下次是第 2 次");
        assert_eq!(v.wait_secs, 2, "第 2 次退避 2s");
        assert_eq!(v.max_attempts, 5);
        assert!(!v.exhausted);
        assert!(v.label.contains("2/5"), "文案含 次/退避: {}", v.label);
    }

    #[test]
    fn exhausted_yields_none() {
        let mut s = ReconnectScheduler::default_policy();
        for _ in 0..5 {
            s.step();
        }
        assert!(s.exhausted());
        assert!(view_of(&s).is_none(), "耗尽后无下次（前端转 NoPath）");
    }

    #[test]
    fn custom_policy_respected() {
        let p = BackoffPolicy {
            base: std::time::Duration::from_secs(1),
            factor: 2,
            max_attempts: 3,
        };
        let s = ReconnectScheduler::new(p);
        let v = view_of(&s).expect("fresh scheduler");
        assert_eq!(v.max_attempts, 3);
        assert_eq!(v.attempt, 1);
        assert_eq!(v.wait_secs, 1);
    }
}
