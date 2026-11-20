use thiserror::Error;

/// 协议层统一错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LpError {
    /// 帧长度字段与剩余字节不符。
    #[error("frame length mismatch: declared {declared}, remaining {remaining}")]
    FrameLenMismatch { declared: usize, remaining: usize },
    /// 帧缓冲区不完整。
    #[error("frame buffer too short: {len} bytes, need {need}")]
    BufferTooShort { len: usize, need: usize },
    /// 协议版本不匹配。
    #[error("unsupported protocol version: {0}")]
    BadVersion(u8),
    /// GCM tag 校验失败（帧被篡改或密钥错误）。
    #[error("gcm tag verification failed")]
    GcmTagInvalid,
    /// 未知报文类型（接收端忽略策略，由上层决定 warn 或丢弃，对齐 protocol.md §7）。
    #[error("unknown message type: 0x{0:02x}")]
    UnknownMsgType(u8),
    /// NALU 分片重组超时或序号乱序（抖动缓冲丢弃，非致命）。
    #[error("nalu reassembly: {0}")]
    Reassembly(String),
    /// 剪贴板超限（对齐 FR-07.2，1MB 上限）。
    #[error("clipboard payload exceeds limit: {0} bytes")]
    ClipboardTooLarge(usize),
    /// bincode 序列化失败。
    #[error("serialize error: {0}")]
    Serialize(String),
    /// bincode 反序列化失败。
    #[error("deserialize error: {0}")]
    Deserialize(String),
}
