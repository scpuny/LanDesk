//! `input` — 键鼠注入（受控端，FR-06）。
//!
//! M1 占位：仅定义 trait + LKC→平台码的映射入口，平台实现（SendInput/uinput/CGEvent）
//! 按 feature 门控，后续里程碑补全。

use lp::keycodes::Lkc;
use thiserror::Error;

/// 注入错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("unsupported key code: {0}")]
    UnsupportedKey(u32),
    #[error("platform backend unavailable: {0}")]
    BackendUnavailable(String),
}

/// 鼠标按键（与 `lp::msg::MouseButton` 编码一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonKind {
    Left,
    Right,
    Middle,
}

/// 键鼠注入抽象。
pub trait InputInjector: Send {
    fn mouse_move(&self, x_norm: u16, y_norm: u16, screen_w: u32, screen_h: u32) -> Result<(), InputError>;
    fn mouse_button(&self, button: MouseButtonKind, down: bool) -> Result<(), InputError>;
    fn mouse_wheel(&self, dx: i16, dy: i16) -> Result<(), InputError>;
    fn key_down(&self, key: u32, mods: u8) -> Result<(), InputError>;
    fn key_up(&self, key: u32, mods: u8) -> Result<(), InputError>;
}

/// 坐标归一化映射（FR-06.3：控制端发 0..=65535 归一化坐标，受控端还原）。
pub fn denormalize(coord_norm: u16, target_dim: u32) -> u32 {
    ((coord_norm as u32) * target_dim) / 65535
}

/// LKC 是否已知（未映射的键码报 [`InputError::UnsupportedKey`]）。
pub fn is_supported_lkc(key: u32) -> bool {
    Lkc::from_u32(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormalize_basic() {
        assert_eq!(denormalize(0, 1080), 0);
        assert_eq!(denormalize(65535, 1080), 1080);
        assert_eq!(denormalize(32768, 1080), 540);
    }

    #[test]
    fn known_keys_supported() {
        assert!(is_supported_lkc(Lkc::A.as_u32()));
        assert!(is_supported_lkc(Lkc::F12.as_u32()));
        assert!(!is_supported_lkc(0x70));
    }
}
