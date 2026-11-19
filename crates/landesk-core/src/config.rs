//! 配置模型（FR-08 / NFR-06，对齐 `docs/architecture.md` §5.1）。
//!
//! 配置项：访问密码（仅存 PBKDF2 派生值，不落明文）、设备名、mDNS 开关、
//! 信令/STUN/中继地址、编码预设、日志级别、脏矩形开关。

use lp::msg::EncodePreset;

/// 日志级别（NFR-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
    Warn,
    Error,
}

/// 受控端配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// 设备名（≤32B）。
    pub device_name: String,
    /// 访问密码（运行时明文；持久化只存派生值，NFR-03）。
    pub password: Option<String>,
    /// 是否广播 mDNS。
    pub mdns_enabled: bool,
    /// 是否注册外网信令（默认关，避免主动暴露公网，NFR-03）。
    pub signaling_enabled: bool,
    /// 信令地址（wss://host:port）。
    pub signaling_url: Option<String>,
    /// STUN 地址（udp://host:port，可指向自建）。
    pub stun_url: Option<String>,
    /// 中继开关（FR-11.2，可关 → 打不通即拒绝）。
    pub relay_enabled: bool,
    /// 编码预设。
    pub preset: EncodePreset,
    /// 脏矩形开关（关则全帧模式，风险表#3）。
    pub dirty_rect: bool,
    /// 日志级别。
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            password: None,
            mdns_enabled: true,
            // 默认不主动暴露公网（NFR-03）
            signaling_enabled: false,
            signaling_url: None,
            stun_url: None,
            relay_enabled: true,
            preset: EncodePreset::Balanced,
            dirty_rect: true,
            log_level: LogLevel::default(),
        }
    }
}
