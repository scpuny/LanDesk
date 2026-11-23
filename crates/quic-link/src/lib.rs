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
//! - [`error`]：错误类型；
//! - [`transport_config`]：统一 QUIC 传输参数（idle 上限 + keepalive PING，防静默 idle 掉线）。

pub mod conn;
pub mod error;
pub mod gcm;
pub mod insecure;
pub mod server;
pub mod client;
pub mod relay;
pub mod tls;

pub use client::QuicClient;
pub use conn::{QuicConnection, VideoDatagramRx, VideoDatagramTx};
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

/// 统一 QUIC 传输参数（[`server`] / [`client`] / [`relay`] 共用）。
///
/// 关键点——**显式开 keepalive**：`TransportConfig::default()` 的 `keep_alive_interval`
/// 为 0（关），仅靠 `max_idle_timeout`（quinn 默认 30s）保活。当视频面因 datagram
/// 限长 drop 无实际流量、用户又暂停操作时，双向控制流 30s 静默即被 QUIC 判 idle
/// 掉线（`connection lost`，写控制流/读控制流双双失败）。远程桌面会话**不允许**
/// 因静默被 idle 掐断，故：
/// - `max_idle_timeout` 显式钉 30s（容忍对端临时断流，与 quinn 默认一致）；
/// - `keep_alive_interval` 10s：本端定期发 PING 包维持连接，**只要一端还活着**，
///   连接就不会因对端静默而 idle 超时（PING 也刷新对端的 idle 计时）。
///
/// datagram 扩展（`datagram_receive_buffer_size`）quinn 0.11 的 `default()` 已默认开启，
/// 此处无需重复设置。
pub fn transport_config() -> quinn::TransportConfig {
    use std::time::Duration;
    let mut t = quinn::TransportConfig::default();
    // 30s 远小于 IdleTimeout 的 VarInt 上限（16777215ms），try_from 必成，unwrap 安全。
    t.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(30)).expect("30s < IdleTimeout 上限"),
    ));
    t.keep_alive_interval(Some(Duration::from_secs(10)));
    t
}

/// 连接方式（FR-09.3 状态栏展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ConnMode {
    /// 内网直连（mDNS 发现）。
    #[default]
    Inner,
    /// 外网 P2P 打洞成功。
    P2p,
    /// 中继转发（FR-11 兜底）。
    Relay,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_conn_mode_is_inner() {
        assert_eq!(ConnMode::default(), ConnMode::Inner);
    }
}
