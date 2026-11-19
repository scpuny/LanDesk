//! 配置页持久化（FR-08，对齐 `docs/architecture.md` §5.1 `~/.config/landesk/config.json`）。
//!
//! 薄封装 [`landesk_core::config::Config`] 的 JSON load/save，加上「默认配置路径」
//! 与「首次运行生成默认模板」语义。纯文件 IO + serde，可离线单测（用临时目录）。
//! Tauri 命令桥（`ui/src-tauri/commands`）调用它实现设置页的读/写。

use std::path::{Path, PathBuf};

use landesk_core::config::{Config, ConfigError};

/// 配置存储错误。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 底层 IO / JSON 失败。
    #[error("config store: {0}")]
    Core(#[from] ConfigError),
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

    /// 读配置；文件不存在 → 返回默认并**生成模板落盘**（§5.1 首次生成）。
    pub fn load_or_init(&self) -> Result<Config, StoreError> {
        if self.path.exists() {
            Ok(Config::load_file(&self.path)?)
        } else {
            let c = Config::default();
            c.save_file(&self.path)?;
            Ok(c)
        }
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
        assert_eq!(c, Config::default(), "首次生成默认模板");
        assert!(p.exists(), "模板已落盘");
        // 二次 load 走 exists 分支，仍一致。
        let c2 = store.load_or_init().expect("reload");
        assert_eq!(c2, c);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_persists_fields() {
        let p = tmp("save.json");
        let _ = std::fs::remove_file(&p);
        let store = ConfigStore::with_path(&p);
        let mut c = Config::default();
        c.device_name = "ok3568-dev".into();
        c.log_level = LogLevel::Debug;
        c.relay_enabled = false;
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
