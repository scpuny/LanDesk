//! `mdns` 错误类型。

use thiserror::Error;

/// 局域网发现错误。
#[derive(Debug, Error)]
pub enum MdnsError {
    /// 底层 mdns-sd 守护 / 注册 / 浏览失败。
    #[error("mdns-sd: {0}")]
    Daemon(#[from] mdns_sd::Error),
    /// 事件通道已断开（守护退出）。
    #[error("mdns event channel closed")]
    ChannelClosed,
    /// 缺少 protocol.md §4 必填 TXT 键。
    #[error("missing required TXT key `{0}`")]
    MissingTxtKey(String),
}
