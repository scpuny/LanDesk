//! 编码器异常重置（FR-04.6 / NFR-01：close + reopen，不直接断会话）。
//!
//! 状态机（对齐 `docs/architecture.md` §4.2 异常恢复矩阵）：
//! ```text
//! Running ──encode error──▶ Reopening ──reopen 成功──▶ Running
//!   │                          │
//!   │ 驱动异常/句柄失效          │ 同档 reopen 连续失败 3 次
//!   ▼                          ▼
//! (标记需重置)             Degrading（降到下一档，最终 OpenH264）
//! ```

/// 编码器运行时状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderState {
    Running,
    /// close 后正在重开同档。
    Reopening,
    /// 同档重开失败次数达阈值，降级到下一档。
    Degrading,
    /// 已降级到软编（openh264）运行。
    SoftFallback,
}

impl EncoderState {
    /// 是否仍可运行（非终态异常）。
    pub const fn is_healthy(&self) -> bool {
        matches!(self, Self::Running | Self::SoftFallback)
    }
}

/// 同档 reopen 最大连续失败次数（达到后降级，对齐 NFR-01）。
pub const MAX_SAME_CLASS_FAILS: u32 = 3;

/// 编码器重置状态机。
#[derive(Debug)]
pub struct ResetController {
    state: EncoderState,
    /// 当前档在同档位下的连续失败计数。
    same_class_fails: u32,
}

impl Default for ResetController {
    fn default() -> Self {
        Self {
            state: EncoderState::Running,
            same_class_fails: 0,
        }
    }
}

impl ResetController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &EncoderState {
        &self.state
    }

    /// 收到一次编码/驱动错误：进入 Reopening；累计同档失败达阈值则 Degrading。
    pub fn on_error(&mut self) {
        self.same_class_fails += 1;
        if self.same_class_fails >= MAX_SAME_CLASS_FAILS {
            self.state = EncoderState::Degrading;
            self.same_class_fails = 0;
        } else {
            self.state = EncoderState::Reopening;
        }
    }

    /// 同档 reopen 成功：回 Running，清零失败计数。
    pub fn on_reopen_ok(&mut self) {
        self.state = EncoderState::Running;
        self.same_class_fails = 0;
    }

    /// 降级到下一档（或软编兜底）成功。
    pub fn on_degraded(&mut self) {
        self.state = EncoderState::SoftFallback;
        self.same_class_fails = 0;
    }

    /// 是否需要重置（Reopening / Degrading）。
    pub fn needs_reset(&self) -> bool {
        matches!(self.state, EncoderState::Reopening | EncoderState::Degrading)
    }

    /// 分辨率变更 / 显示器插拔：强制重置（Reopening，同档）。
    pub fn on_rebuild(&mut self) {
        self.state = EncoderState::Reopening;
        self.same_class_fails = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_then_reopen_ok_recovers() {
        let mut c = ResetController::new();
        assert!(!c.needs_reset());
        c.on_error();
        assert!(c.needs_reset());
        assert_eq!(*c.state(), EncoderState::Reopening);
        c.on_reopen_ok();
        assert_eq!(*c.state(), EncoderState::Running);
        assert!(!c.needs_reset());
    }

    #[test]
    fn repeated_failures_trigger_degrade() {
        let mut c = ResetController::new();
        // 连续 3 次同档失败 → Degrading
        c.on_error();
        assert_eq!(*c.state(), EncoderState::Reopening, "第 1 次失败应 Reopening");
        c.on_error();
        assert_eq!(*c.state(), EncoderState::Reopening, "第 2 次失败应 Reopening");
        c.on_error();
        assert_eq!(*c.state(), EncoderState::Degrading, "第 3 次失败应触发降级");
        c.on_degraded();
        assert_eq!(*c.state(), EncoderState::SoftFallback);
        assert!(c.state().is_healthy());
    }

    #[test]
    fn rebuild_forces_reopen() {
        let mut c = ResetController::new();
        c.on_rebuild();
        assert!(c.needs_reset());
        c.on_reopen_ok();
        assert_eq!(*c.state(), EncoderState::Running);
    }
}
