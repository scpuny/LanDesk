//! `input` — 键鼠注入（受控端，FR-06）。
//!
//! 三平台原生实现按 `target_os` 自动编译（受控端 GUI 无需手动开 feature）：
//! - macOS：CoreGraphics CGEvent（extern "C" FFI，需辅助功能权限）；
//! - Windows：user32 SendInput（windows-sys 0.61.2）；
//! - Linux：uinput（`/dev/uinput`，需 uinput 权限）。
//!
//! [`platform_injector`] 给受控端统一入口：构造平台注入器；无权限时返回
//! [`InputError::BackendUnavailable`]，由调用方降级（只发画面不注入，不影响连接）。

use lp::keycodes::Lkc;
use thiserror::Error;

// 平台实现（cfg 门控；其余平台仅 trait + 映射逻辑，无注入器）。
#[cfg(target_os = "macos")]
mod platform_macos;
#[cfg(target_os = "macos")]
use platform_macos::CGEventInjector as NativeInjector;

#[cfg(target_os = "windows")]
mod platform_windows;
#[cfg(target_os = "windows")]
use platform_windows::SendInputInjector as NativeInjector;

#[cfg(target_os = "linux")]
mod platform_linux;
#[cfg(target_os = "linux")]
use platform_linux::UinputInjector as NativeInjector;

/// 构造当前平台的注入器。非支持平台返回 `BackendUnavailable`。
pub fn platform_injector() -> Result<Box<dyn InputInjector>, InputError> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        NativeInjector::new().map(|n| Box::new(n) as Box<dyn InputInjector>)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(InputError::BackendUnavailable("unsupported platform".into()))
    }
}

/// 主显示器像素尺寸（denormalize 基准）；非支持平台回退 1920×1080。
pub fn primary_screen_size() -> (u32, u32) {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        platform_impl_screen_size()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        (1920, 1080)
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn platform_impl_screen_size() -> (u32, u32) {
    // 平台模块导出的同名函数经 NativeInjector 模块私有，这里用条件别名。
    #[cfg(target_os = "macos")]
    {
        platform_macos::primary_screen_size()
    }
    #[cfg(target_os = "windows")]
    {
        platform_windows::primary_screen_size()
    }
    #[cfg(target_os = "linux")]
    {
        platform_linux::primary_screen_size()
    }
}

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
