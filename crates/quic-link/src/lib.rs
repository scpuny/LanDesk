//! `quic-link` — QUIC 双通道（FR-05，对齐 `docs/protocol.md` §1/§2、`architecture.md` §3）。
//!
//! M3 接入 Quinn：一条 QUIC 连接上同时承载
//! - **可靠控制流**（reliable stream）：`lp` 控制帧（鉴权/输入/剪贴板/编码器控制/心跳）；
//! - **不可靠视频 Datagram**（QUIC datagram）：H.264 NALU 分片，低时延可丢包。
//!
//! 模块划分：
//! - [`tls`]：rcgen 自签证书（内网直连 / 环路测试）；
//! - [`insecure`]：信任任意证书的 QUIC 客户端验证器（LAN 跨机一键连接，保留 TLS1.3 签名校验）；
//! - [`server`] / [`client`]：端点封装；
//! - [`conn`]：双通道连接句柄 [`QuicConnection`]；
//! - [`relay`]：中继模式（FR-11 兜底，ADR-A3 字节透传——ticket 前导 + 控制流泵 + 视频 Datagram 盲搬）；
//! - [`gcm`]：`crypto::GcmSession` 语义的 `lp::wire::Gcm` 适配器 [`LpGcm`]；
//! - [`error`]：错误类型。

pub mod conn;
pub mod error;
pub mod gcm;
pub mod insecure;
pub mod server;
pub mod client;
pub mod relay;
pub mod tls;

pub use client::QuicClient;
pub use conn::QuicConnection;
pub use error::QuicLinkError;
pub use gcm::LpGcm;
pub use insecure::TrustAny;
pub use relay::{
    drive_matched, forward_all, forward_video, pump_control_pair, pump_stream, random_ticket,
    read_ticket, ticket_prefix_bytes, write_ticket, AddOutcome, RelayEndpoint, RelayError,
    RelayMatch, RELAY_STREAM_MAGIC, RELAY_TICKET_CAP,
};
pub use server::QuicServer;
pub use tls::{SelfSigned, TlsError};

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

impl Default for ConnMode {
    fn default() -> Self {
        ConnMode::Inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_conn_mode_is_inner() {
        assert_eq!(ConnMode::default(), ConnMode::Inner);
    }
}
