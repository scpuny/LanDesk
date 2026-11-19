//! 指数退避重连调度（FR-05.4，对齐 `docs/architecture.md` §4.1 状态机 + §4.2 异常恢复矩阵）。
//!
//! QUIC 断连后进入 [`SessionState::Reconnecting`]，按 1s→2s→4s→8s→16s 退避重试，
//! **最多 5 次**；全部失败转 [`SessionState::NoPath`]（打洞失败且无中继 / 中继不可达），
//! 任意一次成功转回 [`SessionState::Running`]（重连成功后强制 IDR，见 §4.2 第 4 行）。
//!
//! 本模块只做**纯调度逻辑**（退避序列 + 状态迁移 + 尝试计数），不含 tokio 定时器——
//! 真实 sleep 由编排层（orchestrator）驱动，这里返回"第 N 次该等多久"，可离线单测。

use std::time::Duration;

use crate::session::{SessionState, IllegalTransition};

/// 退避策略。
///
/// 第 `i` 次重试（`i` 从 0 起）的等待 = `base * factor^i`；共 `max_attempts` 次。
/// 默认 1s 起、×2、最多 5 次 → 1/2/4/8/16s（FR-05.4 的"1/2/4/8s ×5"读法）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffPolicy {
    /// 首次重试等待（默认 1s）。
    pub base: Duration,
    /// 倍增因子（默认 2）。
    pub factor: u32,
    /// 最多重试次数（默认 5，超过即 [`SessionState::NoPath`]）。
    pub max_attempts: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            factor: 2,
            max_attempts: 5,
        }
    }
}

impl BackoffPolicy {
    /// 第 `i` 次重试（0 起）前应等待的时长；`i >= max_attempts` 时已耗尽，返回 `None`。
    pub fn delay_for(&self, i: u32) -> Option<Duration> {
        if i >= self.max_attempts {
            return None;
        }
        let shifted = self.factor.checked_pow(i as u32)?; // 溢出视为耗尽
        Some(self.base.checked_mul(shifted)?)
    }

    /// 完整退避序列（`max_attempts` 项，供 UI 展示 / 断言）。
    pub fn schedule(&self) -> Vec<Duration> {
        (0..self.max_attempts)
            .filter_map(|i| self.delay_for(i))
            .collect()
    }
}

/// 重连调度器（状态 = 已用尝试数 + 策略）。
#[derive(Debug, Clone, Copy)]
pub struct ReconnectScheduler {
    policy: BackoffPolicy,
    used: u32,
}

/// 单次重试推进的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    /// 本次是第几次尝试（1 起）。
    pub attempt: u32,
    /// 本轮前应等待的退避时长（= `policy.delay_for(attempt-1)`）。
    pub wait: Duration,
    /// 推进后的会话状态（`Reconnecting`；耗尽则 `NoPath`）。
    pub state: SessionState,
}

impl ReconnectScheduler {
    /// 起一个新的重连周期（会话从 `Running` 掉线）。
    pub fn new(policy: BackoffPolicy) -> Self {
        Self { policy, used: 0 }
    }

    /// 默认策略（1/2/4/8/16s ×5）。
    pub fn default_policy() -> Self {
        Self::new(BackoffPolicy::default())
    }

    /// 当前已用尝试数。
    pub fn used(&self) -> u32 {
        self.used
    }

    /// 退避策略（只读；UI 视图 / 断言用）。
    pub const fn policy(&self) -> &BackoffPolicy {
        &self.policy
    }

    /// 最多重试次数（= 策略的 `max_attempts`）。
    pub const fn max_attempts(&self) -> u32 {
        self.policy.max_attempts
    }

    /// 是否已耗尽（`used >= max_attempts`，后续转 `NoPath`）。
    pub fn exhausted(&self) -> bool {
        self.used >= self.policy.max_attempts
    }

    /// 当前逻辑状态：耗尽 → `NoPath`；否则 → `Reconnecting`。
    pub fn state(&self) -> SessionState {
        if self.exhausted() {
            SessionState::NoPath
        } else {
            SessionState::Reconnecting
        }
    }

    /// 推进一次重试：登记第 `used+1` 次，返回该次应等待的时长与新状态。
    ///
    /// 返回的 `wait` 供编排层 `sleep`；`state` 供喂 UI 事件（`StateChanged`）。
    pub fn step(&mut self) -> StepOutcome {
        let attempt = self.used + 1;
        self.used = attempt;
        // 语义上：每次重试前会话处于 Reconnecting；耗尽后转 NoPath（编排层据此停止重试）。
        let wait = self
            .policy
            .delay_for(attempt - 1)
            .unwrap_or_default();
        let state = if self.exhausted() {
            SessionState::NoPath
        } else {
            SessionState::Reconnecting
        };
        StepOutcome {
            attempt,
            wait,
            state,
        }
    }

    /// 一次重试成功：复位周期（`NoPath` 之外回到 `Running`，重连后强制 IDR 由编排层发）。
    pub fn reset_on_success(&mut self) -> SessionState {
        self.used = 0;
        SessionState::Running
    }

    /// 把本次重试结果（成功 / 仍断）翻译为 [`SessionState`] 迁移，经
    /// [`SessionState::transition`] 校验合法性（非法迁移返回 [`IllegalTransition`]）。
    ///
    /// `from` 为掉线前状态（正常是 `Running`；若是 `NoPath` 重发起则走 `NoPath→Idle` 另路，
    /// 本助手只覆盖 `Running→Reconnecting→{Running|NoPath}` 一条链）。
    pub fn apply(&mut self, from: SessionState, succeeded: bool) -> Result<SessionState, IllegalTransition> {
        if succeeded {
            let next = self.reset_on_success();
            return from.transition(next);
        }
        let _ = self.step();
        from.transition(self.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_schedule_is_1_2_4_8_16() {
        let p = BackoffPolicy::default();
        assert_eq!(
            p.schedule(),
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
            ]
        );
    }

    #[test]
    fn delay_beyond_max_is_none() {
        let p = BackoffPolicy {
            base: Duration::from_secs(1),
            factor: 2,
            max_attempts: 3,
        };
        assert_eq!(p.delay_for(0), Some(Duration::from_secs(1)));
        assert_eq!(p.delay_for(2), Some(Duration::from_secs(4)));
        assert_eq!(p.delay_for(3), None, "i>=max → 耗尽");
    }

    #[test]
    fn factor_overflow_caps_at_none() {
        // 巨量次数下 factor^i 会溢出 → checked_pow 返 None → 视为耗尽（不 panic）。
        let p = BackoffPolicy {
            base: Duration::from_secs(1),
            factor: 1_000_000,
            max_attempts: 40,
        };
        assert!(p.delay_for(0).is_some());
        assert!(p.delay_for(39).is_none(), "溢出应被截断而非 panic");
    }

    #[test]
    fn step_counts_up_to_exhaustion_then_nopath() {
        let mut s = ReconnectScheduler::default_policy();
        assert_eq!(s.state(), SessionState::Reconnecting);
        let mut waits = Vec::new();
        for _ in 0..5 {
            let o = s.step();
            waits.push(o.wait);
        }
        assert_eq!(
            waits,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
            ]
        );
        assert!(s.exhausted());
        assert_eq!(s.state(), SessionState::NoPath, "5 次耗尽 → NoPath（FR-05.4）");
    }

    #[test]
    fn success_resets_and_runs() {
        let mut s = ReconnectScheduler::default_policy();
        s.step();
        s.step();
        assert_eq!(s.used(), 2);
        assert_eq!(s.reset_on_success(), SessionState::Running);
        assert_eq!(s.used(), 0, "成功复位尝试计数");
        assert_eq!(s.state(), SessionState::Reconnecting);
    }

    #[test]
    fn apply_transitions_through_state_machine() {
        let mut s = ReconnectScheduler::default_policy();
        // Running →（仍断）Reconnecting
        let st = s.apply(SessionState::Running, false).unwrap();
        assert_eq!(st, SessionState::Reconnecting);
        // Reconnecting →（成功）Running
        let st = s.apply(SessionState::Reconnecting, true).unwrap();
        assert_eq!(st, SessionState::Running);
    }

    #[test]
    fn apply_exhaustion_reaches_nopath_via_transition() {
        let mut s = ReconnectScheduler::new(BackoffPolicy {
            base: Duration::from_secs(1),
            factor: 2,
            max_attempts: 2,
        });
        // 第一次：Running→Reconnecting
        assert_eq!(s.apply(SessionState::Running, false).unwrap(), SessionState::Reconnecting);
        // 第二次耗尽：Reconnecting→NoPath
        let st = s.apply(SessionState::Reconnecting, false).unwrap();
        assert_eq!(st, SessionState::NoPath);
        assert!(s.exhausted());
    }

    #[test]
    fn apply_illegal_from_idle_rejected() {
        let mut s = ReconnectScheduler::default_policy();
        // Idle 直接进重连链不合法（须先 Idle→Auth→Running）。
        assert!(s.apply(SessionState::Idle, false).is_err());
    }
}
