//! 会话状态机（FR-05.4，对齐 `docs/architecture.md` §4.1 状态机图）。

/// 会话状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// 未连接。
    Idle,
    /// 鉴权中（ECDH + 密码互验）。
    Auth,
    /// 正常运行（视频/控制流已通）。
    Running,
    /// 断线重连中（指数退避，FR-05.4）。
    Reconnecting,
    /// 鉴权失败（密码错 / 设备不存在）。
    AuthFailed,
    /// 无可用路径（打洞失败且无中继）。
    NoPath,
    /// 已关闭。
    Closed,
}

impl SessionState {
    /// 是否仍可恢复（非终态失败）。
    pub const fn recoverable(&self) -> bool {
        matches!(self, Self::Idle | Self::Auth | Self::Running | Self::Reconnecting)
    }

    /// 是否终态（需要用户重新发起连接）。
    pub const fn is_terminal(&self) -> bool {
        !self.recoverable()
    }
}

/// 会话状态迁移（非法迁移报 `Err`，供编排层校验）。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("illegal session transition: {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: SessionState,
    pub to: SessionState,
}

impl SessionState {
    /// 合法迁移校验（对齐 §4.1 状态机）。
    pub fn transition(&self, to: SessionState) -> Result<SessionState, IllegalTransition> {
        let ok = matches!(
            (self, &to),
            (Self::Idle, Self::Auth)
                | (Self::Auth, Self::Running)
                | (Self::Auth, Self::AuthFailed)
                | (Self::Auth, Self::NoPath)
                | (Self::Running, Self::Reconnecting)
                | (Self::Running, Self::Closed)
                | (Self::Reconnecting, Self::Running)
                | (Self::Reconnecting, Self::NoPath)
                | (Self::Reconnecting, Self::Closed)
                | (Self::AuthFailed, Self::Idle)
                | (Self::NoPath, Self::Idle)
                | (Self::Closed, Self::Idle)
        );
        if ok {
            Ok(to)
        } else {
            Err(IllegalTransition { from: *self, to })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path() {
        let mut s = SessionState::Idle;
        s = s.transition(SessionState::Auth).unwrap();
        s = s.transition(SessionState::Running).unwrap();
        s = s.transition(SessionState::Reconnecting).unwrap();
        s = s.transition(SessionState::Running).unwrap();
        s = s.transition(SessionState::Closed).unwrap();
    }

    #[test]
    fn illegal_idle_to_running_rejected() {
        let s = SessionState::Idle;
        assert!(s.transition(SessionState::Running).is_err());
    }

    #[test]
    fn terminal_states_return_to_idle() {
        for t in [SessionState::AuthFailed, SessionState::NoPath, SessionState::Closed] {
            assert!(t.is_terminal());
            assert!(t.transition(SessionState::Idle).is_ok());
        }
    }
}
