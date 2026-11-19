//! `quic-link` — QUIC 双通道（FR-05，对齐 `docs/protocol.md` §1/§2、`architecture.md` §3）。
//!
//! M1 占位：定义连接模式枚举与通道抽象。M3 接入 Quinn（可靠控制流 + 不可靠视频 Datagram）。

use thiserror::Error;

/// 连接方式（FR-09.3 状态栏展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnMode {
    /// 内网直连（mDNS 发现）。
    Inner,
    /// 外网 P2P 打洞成功。
    P2p,
    /// 中继转发（FR-11 兜底）。
    Relay,
}

/// QUIC 层错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuicLinkError {
    #[error("quic connect failed: {0}")]
    ConnectFailed(String),
    #[error("video dgram send dropped (loss)")]
    VideoDropped,
    #[error("control stream closed")]
    ControlClosed,
}

/// 双通道句柄（M3 实现；M1 仅为占位结构）。
pub struct QuicLink {
    /// 当前连接方式（探测后更新）。
    pub mode: ConnMode,
}

impl QuicLink {
    pub fn new() -> Self {
        Self { mode: ConnMode::Inner }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_conn_mode_is_inner() {
        assert_eq!(QuicLink::new().mode, ConnMode::Inner);
    }
}
