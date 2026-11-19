//! 对外事件通道（喂 UI 状态栏，FR-09.3，对齐 `docs/architecture.md` §3 landesk-core/events.rs）。
//!
//! 控制端状态栏需要实时展示：延迟、编码类型、连接方式（内网/P2P/中继）、丢包率。
//! 本模块定义事件枚举，`landesk-core` 通过 tokio mpsc 向 UI 推送。

use quic_link::ConnMode;

/// 核心层 → UI 的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    /// 连接方式变化（FR-09.3）。
    ConnModeChanged(ConnMode),
    /// 当前编码器类型（`lp::msg::EncoderKind` 的 u8）。
    EncoderChanged(u8),
    /// 端到端延迟（ms，PING/PONG 测得）。
    LatencyMs(u32),
    /// 视频丢包率（百分比，滑动 5s 窗口，FR-05.5）。
    LossPct(u8),
    /// 码率（kbps，编码器动态调整，FR-04.5）。
    BitrateKbps(u32),
    /// 编码器重置事件（FR-04.6）。
    EncoderReset(u8),
    /// 会话状态变化。
    StateChanged(crate::session::SessionState),
}

/// 事件订阅（M4~M8 由 orchestrator 填充）。
pub type EventSender = tokio::sync::mpsc::Sender<CoreEvent>;

/// 创建一对事件通道。
pub fn event_channel(buffer: usize) -> (EventSender, tokio::sync::mpsc::Receiver<CoreEvent>) {
    tokio::sync::mpsc::channel(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_channel_delivers() {
        let (tx, mut rx) = event_channel(4);
        use crate::session::SessionState;
        tx.send(CoreEvent::StateChanged(SessionState::Idle)).await.unwrap();
        assert_eq!(rx.recv().await, Some(CoreEvent::StateChanged(SessionState::Idle)));
    }
}
