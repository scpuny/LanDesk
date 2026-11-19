//! `landesk-core` 编排层错误（M4 一键连接）。

use thiserror::Error;

/// 编排层错误。
#[derive(Debug, Error)]
pub enum CoreError {
    /// mDNS 发现 / 广播失败。
    #[error("mdns: {0}")]
    Mdns(#[from] mdns::MdnsError),
    /// QUIC 通道错误（端点 / 握手 / 控制流 / Datagram）。
    #[error("quic: {0}")]
    Quic(#[from] quic_link::QuicLinkError),
    /// 控制帧加解密（编解码 / GCM tag / 版本）。
    #[error("frame: {0}")]
    Frame(#[from] lp::LpError),
    /// 握手失败（引导密钥不匹配 / tag 校验失败 / 对端拒绝）。
    #[error("handshake failed: {0}")]
    Handshake(String),
    /// 鉴权失败（密码错 / 设备不存在 / 对端关闭）。
    #[error("auth failed: {0}")]
    Auth(String),
    /// 目标不可达（地址无法解析 / 连接被拒）。
    #[error("unreachable: {0}")]
    Unreachable(String),
}
