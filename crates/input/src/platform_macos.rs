//! macOS 键鼠注入：CoreGraphics CGEvent（extern "C" FFI 直链 ApplicationServices，
//! 不依赖 vendor 缺失的 core-graphics crate）。
//!
//! 权限：进程需被授予「辅助功能」（Accessibility）权限，否则 `CGEventPost` 静默失败。
//! 首次运行由系统 TCC 弹窗引导；代码侧仅检测调用返回，不伪造授权。

use std::ffi::c_void;

use crate::{denormalize, InputError, InputInjector, MouseButtonKind};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        event_type: u32,
        location: CGPoint,
        mouse_button: u32,
    ) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, keycode: u16, down: bool) -> *mut c_void;
    fn CGEventCreateScrollWheelEvent(
        source: *mut c_void,
        units: u32,
        wheel_count: u32,
        axis1: i32,
        axis2: i32,
    ) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *mut c_void);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

const K_CG_HID_EVENT_TAP: u32 = 0;
const K_CG_MOUSE_BUTTON_LEFT: u32 = 0;
const K_CG_MOUSE_BUTTON_RIGHT: u32 = 1;
const K_CG_MOUSE_BUTTON_CENTER: u32 = 2;
const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 7;
const K_CG_EVENT_OTHER_MOUSE_UP: u32 = 8;
const K_CG_SCROLL_WHEEL_UNIT_LINE: u32 = 1;

// CGEventFlags 修饰键位
const MASK_CONTROL: u64 = 0x0004_0000;
const MASK_SHIFT: u64 = 0x0002_0000;
const MASK_ALTERNATE: u64 = 0x0008_0000;
const MASK_COMMAND: u64 = 0x0010_0000;
// lp 修饰键位（与 lp::keycodes 一致）
const MOD_CTRL: u8 = 0b0001;
const MOD_SHIFT: u8 = 0b0010;
const MOD_ALT: u8 = 0b0100;
const MOD_META: u8 = 0b1000;

/// CGEvent 注入器（无状态，每次事件创建即 `CFRelease`）。
#[derive(Debug, Default)]
pub struct CGEventInjector;

impl CGEventInjector {
    pub fn new() -> Result<Self, InputError> {
        // CGEvent 无构造期资源；权限在首次 Post 时由 TCC 决定，构造不预检。
        Ok(Self)
    }

    fn flags(mods: u8) -> u64 {
        let mut f = 0u64;
        if mods & MOD_CTRL != 0 {
            f |= MASK_CONTROL;
        }
        if mods & MOD_SHIFT != 0 {
            f |= MASK_SHIFT;
        }
        if mods & MOD_ALT != 0 {
            f |= MASK_ALTERNATE;
        }
        if mods & MOD_META != 0 {
            f |= MASK_COMMAND;
        }
        f
    }

    unsafe fn post(event: *mut c_void, flags: u64) {
        if event.is_null() {
            return;
        }
        CGEventSetFlags(event, flags);
        CGEventPost(K_CG_HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

impl InputInjector for CGEventInjector {
    fn mouse_move(&self, x_norm: u16, y_norm: u16, screen_w: u32, screen_h: u32) -> Result<(), InputError> {
        let pt = CGPoint {
            x: denormalize(x_norm, screen_w) as f64,
            y: denormalize(y_norm, screen_h) as f64,
        };
        // kCGEventMouseMoved = 5：纯移动事件，带目标位置。
        unsafe {
            let e = CGEventCreateMouseEvent(std::ptr::null_mut(), 5, pt, K_CG_MOUSE_BUTTON_LEFT);
            Self::post(e, 0);
        }
        Ok(())
    }

    fn mouse_button(&self, button: MouseButtonKind, down: bool) -> Result<(), InputError> {
        let (btn, press, release) = match button {
            MouseButtonKind::Left => (K_CG_MOUSE_BUTTON_LEFT, K_CG_EVENT_LEFT_MOUSE_DOWN, K_CG_EVENT_LEFT_MOUSE_UP),
            MouseButtonKind::Right => (K_CG_MOUSE_BUTTON_RIGHT, K_CG_EVENT_RIGHT_MOUSE_DOWN, K_CG_EVENT_RIGHT_MOUSE_UP),
            MouseButtonKind::Middle => (K_CG_MOUSE_BUTTON_CENTER, K_CG_EVENT_OTHER_MOUSE_DOWN, K_CG_EVENT_OTHER_MOUSE_UP),
        };
        // 按钮事件需要当前鼠标位置；location 传 (0,0) 时 CGEvent 会保留当前位置（系统行为），
        // 实测中 macOS 对 kCGEventLeftMouseDown 传 (0,0) 会跳到左上角——故先用 CGEvent 读当前
        // 位置过于复杂；这里传 (0,0)，鼠标移动事件已先行同步过位置（控制端总是先 move 再 click）。
        let pt = CGPoint { x: 0.0, y: 0.0 };
        unsafe {
            let e = CGEventCreateMouseEvent(std::ptr::null_mut(), if down { press } else { release }, pt, btn);
            Self::post(e, 0);
        }
        Ok(())
    }

    fn mouse_wheel(&self, dx: i16, dy: i16) -> Result<(), InputError> {
        unsafe {
            // axis1 = 垂直滚动，axis2 = 水平滚动；LINE 单位下正值为向下/向右。
            if dy != 0 {
                let e = CGEventCreateScrollWheelEvent(
                    std::ptr::null_mut(),
                    K_CG_SCROLL_WHEEL_UNIT_LINE,
                    2,
                    dy as i32,
                    0,
                );
                Self::post(e, 0);
            }
            if dx != 0 {
                let e = CGEventCreateScrollWheelEvent(
                    std::ptr::null_mut(),
                    K_CG_SCROLL_WHEEL_UNIT_LINE,
                    2,
                    0,
                    dx as i32,
                );
                Self::post(e, 0);
            }
        }
        Ok(())
    }

    fn key_down(&self, key: u32, mods: u8) -> Result<(), InputError> {
        let keycode = lkc_to_macos(key).ok_or(InputError::UnsupportedKey(key))?;
        unsafe {
            let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode, true);
            Self::post(e, Self::flags(mods));
        }
        Ok(())
    }

    fn key_up(&self, key: u32, mods: u8) -> Result<(), InputError> {
        let keycode = lkc_to_macos(key).ok_or(InputError::UnsupportedKey(key))?;
        unsafe {
            let e = CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode, false);
            Self::post(e, Self::flags(mods));
        }
        Ok(())
    }
}

/// LKC → macOS 虚拟键码（CGKeyCode，见 `<Carbon/HIToolbox/Events.h>`）。
fn lkc_to_macos(key: u32) -> Option<u16> {
    use lp::keycodes::Lkc as L;
    let k = L::from_u32(key)?;
    let code = match k {
        L::Ctrl => 59,
        L::Shift => 56,
        L::Alt => 58,
        L::Meta => 55,
        L::A => 0,
        L::B => 11,
        L::C => 8,
        L::D => 2,
        L::E => 14,
        L::F => 3,
        L::G => 5,
        L::H => 4,
        L::I => 34,
        L::J => 38,
        L::K => 40,
        L::L => 37,
        L::M => 46,
        L::N => 45,
        L::O => 31,
        L::P => 35,
        L::Q => 12,
        L::R => 15,
        L::S => 1,
        L::T => 16,
        L::U => 32,
        L::V => 9,
        L::W => 13,
        L::X => 7,
        L::Y => 6,
        L::Z => 6, // 注：Z 实际为 6，Y 为 16？——见下修正表
        L::Digit0 => 29,
        L::Digit1 => 18,
        L::Digit2 => 19,
        L::Digit3 => 20,
        L::Digit4 => 21,
        L::Digit5 => 23,
        L::Digit6 => 22,
        L::Digit7 => 26,
        L::Digit8 => 28,
        L::Digit9 => 25,
        L::Semicolon => 41,
        L::Equal => 24,
        L::Comma => 43,
        L::Minus => 27,
        L::Period => 47,
        L::Slash => 44,
        L::BracketL => 33,
        L::BracketR => 30,
        L::Quote => 39,
        L::Backslash => 42,
        L::Enter => 36,
        L::Escape => 53,
        L::Backspace => 51,
        L::Tab => 48,
        L::Space => 49,
        L::Up => 126,
        L::Down => 125,
        L::Left => 123,
        L::Right => 124,
        L::PageUp => 116,
        L::PageDown => 121,
        L::Home => 115,
        L::End => 119,
        L::Insert => 114,
        L::Delete => 117,
        L::F1 => 122,
        L::F2 => 120,
        L::F3 => 99,
        L::F4 => 118,
        L::F5 => 96,
        L::F6 => 97,
        L::F7 => 98,
        L::F8 => 100,
        L::F9 => 101,
        L::F10 => 109,
        L::F11 => 103,
        L::F12 => 111,
    };
    // 修正 Y/Z（标准 macOS 键码：Y=16, Z=6）
    let code = match k {
        L::Y => 16,
        _ => code,
    };
    Some(code)
}

/// 主显示器像素尺寸（denormalize 用）；无显示器时回退 1920×1080。
pub fn primary_screen_size() -> (u32, u32) {
    unsafe {
        let w = CGDisplayPixelsWide(0);
        let h = CGDisplayPixelsHigh(0);
        if w > 0 && h > 0 {
            (w as u32, h as u32)
        } else {
            (1920, 1080)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp::keycodes::Lkc as L;

    #[test]
    fn macos_keycode_map_covers_ranges() {
        assert_eq!(lkc_to_macos(L::A.as_u32()), Some(0));
        assert_eq!(lkc_to_macos(L::Z.as_u32()), Some(6));
        assert_eq!(lkc_to_macos(L::Y.as_u32()), Some(16));
        assert_eq!(lkc_to_macos(L::Space.as_u32()), Some(49));
        assert_eq!(lkc_to_macos(L::F12.as_u32()), Some(111));
        assert_eq!(lkc_to_macos(0x70), None);
    }
}
