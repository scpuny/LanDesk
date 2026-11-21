//! 配置页持久化（FR-08，对齐 `docs/architecture.md` §5.1 `~/.config/landesk/config.json`）。
//!
//! 薄封装 [`landesk_core::config::Config`] 的 JSON load/save，加上「默认配置路径」
//! 与「首次运行生成默认模板」语义。纯文件 IO + serde，可离线单测（用临时目录）。
//! Tauri 命令桥（`ui/src-tauri/commands`）调用它实现设置页的读/写。

use std::path::{Path, PathBuf};

use landesk_core::config::{Config, ConfigError};

/// 配置存储错误。
///
/// 手写 `Serialize`（序列化为错误字符串）：Tauri 命令出参的 error 类型须可序列化，
/// 而变体包裹的 `std::io::Error` / `serde_json::Error` 本身非 `Serialize`，derive 不适用。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 底层 IO / JSON 失败。
    #[error("config store: {0}")]
    Core(#[from] ConfigError),
}

impl serde::Serialize for StoreError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// 默认配置路径（`~/.config/landesk/config.json`，§5.1；`$XDG_CONFIG_HOME` 优先）。
pub fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("landesk").join("config.json");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| ".".into());
    PathBuf::from(home)
        .join(".config")
        .join("landesk")
        .join("config.json")
}

/// 读取本机机器名（首次运行的默认设备名）。
///
/// - Windows：`COMPUTERNAME`（系统环境变量，无额外依赖）；
/// - Unix（macOS/Linux）：`gethostname(2)`（vendor 已含 libc）。
///
/// 读不到返回 `None`——调用方保留空串，交给用户在设置页手填。
fn detect_machine_name() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(windows))]
    {
        let mut buf = [0_i8; 256];
        // SAFETY: buf 是 256 字节可写栈缓冲，长度按容量传；成功时内核写入 NUL 结尾字符串。
        if unsafe { libc::gethostname(buf.as_mut_ptr(), buf.len()) } != 0 {
            return None;
        }
        let bytes: Vec<u8> = buf.iter().take_while(|&&b| b != 0).map(|&b| b as u8).collect();
        let name = String::from_utf8_lossy(&bytes).trim().to_string();
        if name.is_empty() { None } else { Some(name) }
    }
}

/// 配置页读/写器（路径固定）。
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    /// 用默认路径构造。
    pub fn new() -> Self {
        Self {
            path: default_config_path(),
        }
    }

    /// 用自定义路径构造（测试 / 命令行覆盖）。
    pub fn with_path(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读配置；文件不存在 → 生成模板落盘（§5.1 首次生成）。
    ///
    /// 设备名为空（首次运行，或老配置从未填过）时用本机机器名回填并落盘——
    /// 设置页与本机卡都直接读该字段，且设备码由它派生，空名会让各机器设备码相同。
    /// 用户已改过的名称非空，不会被覆盖。
    pub fn load_or_init(&self) -> Result<Config, StoreError> {
        let mut dirty = !self.path.exists();
        let mut c = if dirty {
            Config::default()
        } else {
            Config::load_file(&self.path)?
        };
        if c.device_name.trim().is_empty() {
            if let Some(name) = detect_machine_name() {
                // 设置页 maxlength=32；机器名超长时截断，避免前端展示被截而落盘超长。
                c.device_name = name.chars().take(32).collect();
                dirty = true;
            }
        }
        if dirty {
            c.save_file(&self.path)?;
        }
        Ok(c)
    }

    /// 保存当前配置。
    pub fn save(&self, c: &Config) -> Result<(), StoreError> {
        c.save_file(&self.path)?;
        Ok(())
    }
}

impl Default for ConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use landesk_core::config::LogLevel;
    use std::env::temp_dir;

    fn tmp(name: &str) -> PathBuf {
        temp_dir()
            .join(format!("landesk-store-test-{}", std::process::id()))
            .join(name)
    }

    #[test]
    fn load_or_init_creates_template_when_missing() {
        let p = tmp("init.json");
        let _ = std::fs::remove_file(&p);
        let store = ConfigStore::with_path(&p);
        let c = store.load_or_init().expect("init");
        // 首次生成：默认模板 + 本机机器名回填（取不到机器名时保持空）。
        let mut expected = Config::default();
        if let Some(name) = detect_machine_name() {
            expected.device_name = name.chars().take(32).collect();
        }
        assert_eq!(c, expected, "首次生成默认模板（设备名取本机机器名）");
        assert!(p.exists(), "模板已落盘");
        // 二次 load 走 exists 分支，仍一致。
        let c2 = store.load_or_init().expect("reload");
        assert_eq!(c2, c);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_or_init_keeps_user_edited_device_name() {
        let p = tmp("edited.json");
        let _ = std::fs::remove_file(&p);
        let store = ConfigStore::with_path(&p);
        store
            .save(&Config {
                device_name: "my-workstation".into(),
                ..Default::default()
            })
            .expect("save edited name");
        let c = store.load_or_init().expect("reload");
        assert_eq!(c.device_name, "my-workstation", "用户改过的设备名不被机器名覆盖");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_persists_fields() {
        let p = tmp("save.json");
        let _ = std::fs::remove_file(&p);
        let store = ConfigStore::with_path(&p);
        let c = Config {
            device_name: "ok3568-dev".into(),
            log_level: LogLevel::Debug,
            relay_enabled: false,
            ..Default::default()
        };
        store.save(&c).expect("save");
        let back = Config::load_file(&p).expect("load back");
        assert_eq!(back.device_name, "ok3568-dev");
        assert_eq!(back.log_level, LogLevel::Debug);
        assert!(!back.relay_enabled, "中继关（FR-11.2 可关）");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn default_path_is_under_config_dir() {
        let p = default_config_path();
        // 至少含 landesk 与 config.json 两级（HOME/XDG 前缀不定）。
        let s = p.to_string_lossy();
        assert!(s.contains("landesk"), "路径含 landesk 目录: {s}");
        assert!(s.ends_with("config.json"));
    }
}
