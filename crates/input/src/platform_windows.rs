//! Windows 键鼠注入：user32 SendInput（windows-sys 0.61.2，vendor 已含）。
//!
//! 鼠标用 `MOUSEEVENTF_ABSOLUTE` 归一化坐标（0..65535 映射全屏，天然契合 LKC
//! 归一化坐标），无需知道屏幕分辨率；滚轮用 WHEEL/HWHEEL（120 = 一格）。
//! 键盘用虚拟键码（VK_*）+ KEYEVENTF_KEYUP 区分按下/抬起。

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEINPUT,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
};

use crate::{InputError, InputInjector, MouseButtonKind};

/// Windows SendInput 注入器（无状态）。
#[derive(Debug, Default)]
pub struct SendInputInjector;

impl SendInputInjector {
    pub fn new() -> Result<Self, InputError> {
        Ok(Self)
    }

    fn send_one(input: INPUT) -> Result<(), InputError> {
        // SendInput 返回实际插入的事件数；0 表示被 UIPI 拦截（低完整性进程不能注入高完整性窗口）。
        let n = unsafe { SendInput(1, &input, core::mem::size_of::<INPUT>() as i32) };
        if n == 0 {
            Err(InputError::BackendUnavailable("SendInput blocked (UIPI)".into()))
        } else {
            Ok(())
        }
    }

    fn mouse_input(flags: u32, dx: i32, dy: i32, data: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard_input(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

impl InputInjector for SendInputInjector {
    fn mouse_move(&self, x_norm: u16, y_norm: u16, _screen_w: u32, _screen_h: u32) -> Result<(), InputError> {
        // ABSOLUTE 模式：dx/dy 是 0..65535 的全屏归一化坐标，直接用 LKC 值。
        Self::send_one(Self::mouse_input(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
            x_norm as i32,
            y_norm as i32,
            0,
        ))
    }

    fn mouse_button(&self, button: MouseButtonKind, down: bool) -> Result<(), InputError> {
        let flag = match (button, down) {
            (MouseButtonKind::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButtonKind::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButtonKind::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButtonKind::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButtonKind::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButtonKind::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        Self::send_one(Self::mouse_input(flag, 0, 0, 0))
    }

    fn mouse_wheel(&self, dx: i16, dy: i16) -> Result<(), InputError> {
        // WHEEL 的 mouseData 为有符号位移（120 = 一格）；HWHEEL 走 dx。
        if dy != 0 {
            Self::send_one(Self::mouse_input(
                MOUSEEVENTF_WHEEL,
                0,
                0,
                (dy as i32 * 120) as u32,
            ))?;
        }
        if dx != 0 {
            Self::send_one(Self::mouse_input(
                MOUSEEVENTF_HWHEEL,
                0,
                0,
                (dx as i32 * 120) as u32,
            ))?;
        }
        Ok(())
    }

    fn key_down(&self, key: u32, _mods: u8) -> Result<(), InputError> {
        let vk = lkc_to_vk(key).ok_or(InputError::UnsupportedKey(key))?;
        Self::send_one(Self::keyboard_input(vk, false))
    }

    fn key_up(&self, key: u32, _mods: u8) -> Result<(), InputError> {
        let vk = lkc_to_vk(key).ok_or(InputError::UnsupportedKey(key))?;
        Self::send_one(Self::keyboard_input(vk, true))
    }
}

/// LKC → Windows 虚拟键码（VK_*，winuser.h）。
fn lkc_to_vk(key: u32) -> Option<u16> {
    use lp::keycodes::Lkc as L;
    let k = L::from_u32(key)?;
    let vk = match k {
        L::Ctrl => 0x11,      // VK_CONTROL
        L::Shift => 0x10,     // VK_SHIFT
        L::Alt => 0x12,       // VK_MENU
        L::Meta => 0x5B,      // VK_LWIN
        L::A => b'A' as u16,
        L::B => b'B' as u16,
        L::C => b'C' as u16,
        L::D => b'D' as u16,
        L::E => b'E' as u16,
        L::F => b'F' as u16,
        L::G => b'G' as u16,
        L::H => b'H' as u16,
        L::I => b'I' as u16,
        L::J => b'J' as u16,
        L::K => b'K' as u16,
        L::L => b'L' as u16,
        L::M => b'M' as u16,
        L::N => b'N' as u16,
        L::O => b'O' as u16,
        L::P => b'P' as u16,
        L::Q => b'Q' as u16,
        L::R => b'R' as u16,
        L::S => b'S' as u16,
        L::T => b'T' as u16,
        L::U => b'U' as u16,
        L::V => b'V' as u16,
        L::W => b'W' as u16,
        L::X => b'X' as u16,
        L::Y => b'Y' as u16,
        L::Z => b'Z' as u16,
        L::Digit0 => b'0' as u16,
        L::Digit1 => b'1' as u16,
        L::Digit2 => b'2' as u16,
        L::Digit3 => b'3' as u16,
        L::Digit4 => b'4' as u16,
        L::Digit5 => b'5' as u16,
        L::Digit6 => b'6' as u16,
        L::Digit7 => b'7' as u16,
        L::Digit8 => b'8' as u16,
        L::Digit9 => b'9' as u16,
        L::Semicolon => 0xBA, // VK_OEM_1
        L::Equal => 0xBB,     // VK_OEM_PLUS
        L::Comma => 0xBC,     // VK_OEM_COMMA
        L::Minus => 0xBD,     // VK_OEM_MINUS
        L::Period => 0xBE,    // VK_OEM_PERIOD
        L::Slash => 0xBF,     // VK_OEM_2
        L::BracketL => 0xDB,  // VK_OEM_4
        L::Backslash => 0xDC, // VK_OEM_5
        L::BracketR => 0xDD,  // VK_OEM_6
        L::Quote => 0xDE,     // VK_OEM_7
        L::Enter => 0x0D,     // VK_RETURN
        L::Escape => 0x1B,    // VK_ESCAPE
        L::Backspace => 0x08, // VK_BACK
        L::Tab => 0x09,       // VK_TAB
        L::Space => 0x20,     // VK_SPACE
        L::Up => 0x26,
        L::Down => 0x28,
        L::Left => 0x25,
        L::Right => 0x27,
        L::PageUp => 0x21,    // VK_PRIOR
        L::PageDown => 0x22,  // VK_NEXT
        L::Home => 0x24,
        L::End => 0x23,
        L::Insert => 0x2D,
        L::Delete => 0x2E,
        L::F1 => 0x70,
        L::F2 => 0x71,
        L::F3 => 0x72,
        L::F4 => 0x73,
        L::F5 => 0x74,
        L::F6 => 0x75,
        L::F7 => 0x76,
        L::F8 => 0x77,
        L::F9 => 0x78,
        L::F10 => 0x79,
        L::F11 => 0x7A,
        L::F12 => 0x7B,
    };
    Some(vk)
}

/// 主显示器像素尺寸（Windows 全局坐标用；ABSOLUTE 注入本身不依赖此值，预留给采屏对齐）。
pub fn primary_screen_size() -> (u32, u32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
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
    fn windows_vk_map_covers_ranges() {
        assert_eq!(lkc_to_vk(L::A.as_u32()), Some(b'A' as u16));
        assert_eq!(lkc_to_vk(L::Z.as_u32()), Some(b'Z' as u16));
        assert_eq!(lkc_to_vk(L::Digit0.as_u32()), Some(b'0' as u16));
        assert_eq!(lkc_to_vk(L::F12.as_u32()), Some(0x7B));
        assert_eq!(lkc_to_vk(L::Space.as_u32()), Some(0x20));
        assert_eq!(lkc_to_vk(0x70), None);
    }
}
