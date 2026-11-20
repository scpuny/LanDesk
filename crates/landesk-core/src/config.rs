//! 配置模型（FR-08 / NFR-06，对齐 `docs/architecture.md` §5.1）。
//!
//! 配置项：访问密码（仅存 PBKDF2 派生值，不落明文）、设备名、mDNS 开关、
//! 信令/STUN/中继地址、编码预设、日志级别、脏矩形开关。

use lp::msg::EncodePreset;
use serde::{Deserialize, Serialize};

/// 日志级别（NFR-06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
    Warn,
    Error,
}

/// 受控端配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// 中继 / TURN 地址（`turn://host:3479`；对齐前端 `LanDeskConfig.relay_url`，
    /// 旧配置无此字段时 serde 对 `Option` 缺省为 `None`，向后兼容）。
    #[serde(default)]
    pub relay_url: Option<String>,
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
            relay_url: None,
            preset: EncodePreset::Balanced,
            dirty_rect: true,
            log_level: LogLevel::default(),
        }
    }
}

// ---------- JSON 持久化（FR-08 配置页 / NFR-06 可观测）----------

/// 配置持久化错误。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// 文件读写失败。
    #[error("config io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON 序列化 / 反序列化失败。
    #[error("config json: {0}")]
    Json(#[from] serde_json::Error),
}

impl Config {
    /// 序列化为 JSON 字符串（配置页「保存」落盘用）。
    pub fn to_json(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string(self)?)
    }

    /// 从 JSON 字符串反序列化。
    pub fn from_json(raw: &str) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(raw)?)
    }

    /// 从文件读配置（文件不存在 → 返回 [`Config::default`]，首次生成默认模板，§5.1）。
    pub fn load_file(path: &std::path::Path) -> Result<Self, ConfigError> {
        match std::fs::read(path) {
            Ok(bytes) => Self::from_json(&String::from_utf8_lossy(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// 写配置到文件（自动建父目录）。
    pub fn save_file(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    /// 隔离的临时文件路径（按进程号避免并发测试冲突；`save_file` 自动建父目录）。
    fn tmp(name: &str) -> std::path::PathBuf {
        temp_dir()
            .join(format!("landesk-cfg-test-{}", std::process::id()))
            .join(name)
    }

    #[test]
    fn default_config_is_public_off_and_relay_on() {
        let c = Config::default();
        assert!(!c.signaling_enabled, "默认不主动暴露公网（NFR-03）");
        assert!(c.relay_enabled, "默认中继开（FR-11.2 可选）");
        assert!(c.mdns_enabled, "默认内网 mDNS 发现（FR-01）");
        assert_eq!(c.preset, EncodePreset::Balanced);
    }

    #[test]
    fn json_roundtrip_preserves_fields() {
        let c = Config {
            device_name: "devboard-ok3568".into(),
            password: Some("s3cr3t".into()),
            preset: EncodePreset::Hd,
            dirty_rect: false,
            log_level: LogLevel::Debug,
            ..Default::default()
        };
        let s = c.to_json().expect("serialize");
        let back = Config::from_json(&s).expect("deserialize");
        assert_eq!(back, c, "JSON 往返字段一致");
    }

    #[test]
    fn save_then_load_file() {
        let path = tmp("config.json");
        let c = Config {
            device_name: "mac-mini".into(),
            ..Default::default()
        };
        c.save_file(&path).expect("save");
        let loaded = Config::load_file(&path).expect("load");
        assert_eq!(loaded, c, "落盘后读回一致");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file_yields_default() {
        let path = tmp("no-such.json");
        let c = Config::load_file(&path).expect("load missing → default");
        assert_eq!(c, Config::default(), "缺文件回填默认（§5.1 首次生成模板）");
    }
}

