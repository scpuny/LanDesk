//! `quic-link` 错误类型。

use thiserror::Error;

/// QUIC 层 / 双通道错误。
#[derive(Debug, Error)]
pub enum QuicLinkError {
    // ---------- 连接建立 ----------
    /// 服务端 / 客户端 endpoint 创建失败（绑定 socket 失败等）。
    #[error("quic endpoint init failed: {0}")]
    EndpointInit(String),
    /// 连接握手失败（`Connecting` 等待超时 / 被拒 / TLS 错）。
    #[error("quic connect failed: {0}")]
    ConnectFailed(String),
    /// 等待对端连接（`accept`）出错。
    #[error("quic accept failed: {0}")]
    AcceptFailed(String),

    // ---------- 可靠控制流 ----------
    /// 打开双向流失败。
    #[error("control stream open failed: {0}")]
    ControlOpen(String),
    /// 写控制帧失败。
    #[error("control stream write failed: {0}")]
    ControlWrite(String),
    /// 读控制帧失败（非流关闭情形）。
    #[error("control stream read failed: {0}")]
    ControlRead(String),
    /// 控制流已关闭（对端 finish / reset / 连接丢失）——正常会话结束信号。
    #[error("control stream closed")]
    ControlClosed,

    // ---------- 不可靠视频 Datagram ----------
    /// 视频 Datagram 发送被丢弃（超对端容量 / 连接已关）。上层降级处理。
    #[error("video datagram dropped (capacity or closed)")]
    VideoDropped,
    /// 视频 Datagram 接收出错（非连接关闭情形）。
    #[error("video datagram recv failed: {0}")]
    VideoRecv(String),
}
