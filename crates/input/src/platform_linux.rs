//! Linux 键鼠注入：uinput（`/dev/uinput` + ioctl，libc vendor 已含）。
//!
//! 创建虚拟 HID 键鼠设备（经典 `uinput_user_dev` 写 absmin/absmax），写 `input_event`
//! 注入绝对坐标（EV_ABS ABS_X/ABS_Y）、按键（EV_KEY）、滚轮（EV_REL），SYN_REPORT 同步。
//!
//! 权限：进程需可读写 `/dev/uinput`（root 或 uinput 组 + udev 规则）。无权限时
//! `UinputInjector::new()` 返回 `BackendUnavailable`，受控端降级为「只发画面不注入」，
//! 不影响连接。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::Mutex;

use libc::{c_int, c_ulong, ioctl, uinput_user_dev};

use crate::{denormalize, InputError, InputInjector, MouseButtonKind};

// linux/uinput.h ioctl 编码（x86_64/aarch64 一致）
const UINPUT_IOCTL_BASE: c_ulong = b'U' as c_ulong;
const UI_SET_EVBIT: c_ulong = _iow(UINPUT_IOCTL_BASE, 100, 4);
const UI_SET_KEYBIT: c_ulong = _iow(UINPUT_IOCTL_BASE, 101, 4);
const UI_SET_ABSBIT: c_ulong = _iow(UINPUT_IOCTL_BASE, 103, 4);
const UI_SET_RELBIT: c_ulong = _iow(UINPUT_IOCTL_BASE, 102, 4);
const UI_DEV_CREATE: c_ulong = _io(UINPUT_IOCTL_BASE, 105);
const UI_DEV_DESTROY: c_ulong = _io(UINPUT_IOCTL_BASE, 106);

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const REL_WHEEL: u16 = 0x08;
const REL_HWHEEL: u16 = 0x06;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

const _IOC_WRITE: c_ulong = 2 << 30;

const fn _ioc(dir: c_ulong, ty: c_ulong, nr: c_ulong, size: c_ulong) -> c_ulong {
    (dir << 30) | (size << 16) | (ty << 8) | nr
}
const fn _io(ty: c_ulong, nr: c_ulong) -> c_ulong {
    _ioc(0, ty, nr, 0)
}
const fn _iow(ty: c_ulong, nr: c_ulong, size: c_ulong) -> c_ulong {
    _ioc(_IOC_WRITE, ty, nr, size)
}

/// `struct input_event`（linux/input-event-codes.h，64 位 timeval）。
#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    r#type: u16,
    code: u16,
    value: i32,
}

/// uinput 注入器：持虚拟设备 fd + 屏幕分辨率（denormalize 基准）。
pub struct UinputInjector {
    file: File,
    screen: Mutex<(u32, u32)>,
}

impl UinputInjector {
    /// 打开 `/dev/uinput` 并建立虚拟键鼠；无权限 / 无设备返回 `BackendUnavailable`。
    pub fn new() -> Result<Self, InputError> {
        let mut file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| {
                InputError::BackendUnavailable(format!("open /dev/uinput failed: {e}"))
            })?;
        let raw = file.as_raw_fd();

        unsafe {
            for ev in [EV_SYN, EV_KEY, EV_ABS, EV_REL] {
                check_ioctl(ioctl(raw, UI_SET_EVBIT, ev as c_int), "SET_EVBIT")?;
            }
            for code in BTN_LEFT..=BTN_MIDDLE {
                check_ioctl(ioctl(raw, UI_SET_KEYBIT, code as c_int), "SET_KEYBIT")?;
            }
            for code in 1..=88u16 {
                check_ioctl(ioctl(raw, UI_SET_KEYBIT, code as c_int), "SET_KEYBIT")?;
            }
            for abs in [ABS_X, ABS_Y] {
                check_ioctl(ioctl(raw, UI_SET_ABSBIT, abs as c_int), "SET_ABSBIT")?;
            }
            for rel in [REL_WHEEL, REL_HWHEEL] {
                check_ioctl(ioctl(raw, UI_SET_RELBIT, rel as c_int), "SET_RELBIT")?;
            }

            // 经典 uinput_user_dev：写 name + ABS 坐标范围 0..65535。
            let mut dev: uinput_user_dev = core::mem::zeroed();
            let name = b"LanDesk-VNC";
            for (i, &b) in name.iter().enumerate() {
                dev.name[i] = b as i8;
            }
            dev.id.bustype = 0x03; // BUS_USB
            dev.id.vendor = 0x1;
            dev.id.product = 0x1;
            dev.absmax[ABS_X as usize] = 65535;
            dev.absmax[ABS_Y as usize] = 65535;
            let bytes = core::slice::from_raw_parts(
                &dev as *const uinput_user_dev as *const u8,
                core::mem::size_of::<uinput_user_dev>(),
            );
            file.write_all(bytes)
                .map_err(|e| InputError::BackendUnavailable(format!("write uinput_user_dev: {e}")))?;

            check_ioctl(ioctl(raw, UI_DEV_CREATE), "DEV_CREATE")?;
        }

        let (w, h) = primary_screen_size();
        Ok(Self {
            file,
            screen: Mutex::new((w, h)),
        })
    }

    fn emit(&self, ev: InputEvent) -> Result<(), InputError> {
        let mut file = &self.file;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &ev as *const InputEvent as *const u8,
                core::mem::size_of::<InputEvent>(),
            )
        };
        file.write_all(bytes)
            .map_err(|e| InputError::BackendUnavailable(e.to_string()))
    }

    fn key(&self, code: u16, down: bool) -> Result<(), InputError> {
        self.emit(InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            r#type: EV_KEY,
            code,
            value: if down { 1 } else { 0 },
        })?;
        self.emit(InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            r#type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        })
    }
}

impl Drop for UinputInjector {
    fn drop(&mut self) {
        unsafe {
            ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

fn check_ioctl(ret: c_int, what: &str) -> Result<(), InputError> {
    if ret < 0 {
        Err(InputError::BackendUnavailable(format!("uinput {what} failed")))
    } else {
        Ok(())
    }
}

impl InputInjector for UinputInjector {
    fn mouse_move(&self, x_norm: u16, y_norm: u16, screen_w: u32, screen_h: u32) -> Result<(), InputError> {
        let x = denormalize(x_norm, screen_w) as i32;
        let y = denormalize(y_norm, screen_h) as i32;
        *self.screen.lock().unwrap() = (screen_w, screen_h);
        // ABS 范围 0..65535：像素 → 范围线性映射。
        let abs_x = ((x.max(0) as i64 * 65535) / screen_w.max(1) as i64) as i32;
        let abs_y = ((y.max(0) as i64 * 65535) / screen_h.max(1) as i64) as i32;
        self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_ABS, code: ABS_X, value: abs_x })?;
        self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_ABS, code: ABS_Y, value: abs_y })?;
        self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_SYN, code: SYN_REPORT, value: 0 })
    }

    fn mouse_button(&self, button: MouseButtonKind, down: bool) -> Result<(), InputError> {
        let code = match button {
            MouseButtonKind::Left => BTN_LEFT,
            MouseButtonKind::Right => BTN_RIGHT,
            MouseButtonKind::Middle => BTN_MIDDLE,
        };
        self.key(code, down)
    }

    fn mouse_wheel(&self, dx: i16, dy: i16) -> Result<(), InputError> {
        if dy != 0 {
            self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_REL, code: REL_WHEEL, value: dy as i32 })?;
        }
        if dx != 0 {
            self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_REL, code: REL_HWHEEL, value: dx as i32 })?;
        }
        self.emit(InputEvent { tv_sec: 0, tv_usec: 0, r#type: EV_SYN, code: SYN_REPORT, value: 0 })
    }

    fn key_down(&self, key: u32, _mods: u8) -> Result<(), InputError> {
        let code = lkc_to_evdev(key).ok_or(InputError::UnsupportedKey(key))?;
        self.key(code, true)
    }

    fn key_up(&self, key: u32, _mods: u8) -> Result<(), InputError> {
        let code = lkc_to_evdev(key).ok_or(InputError::UnsupportedKey(key))?;
        self.key(code, false)
    }
}

/// LKC → Linux evdev key code（linux/input-event-codes.h）。
fn lkc_to_evdev(key: u32) -> Option<u16> {
    use lp::keycodes::Lkc as L;
    let k = L::from_u32(key)?;
    let code = match k {
        L::Ctrl => 29,
        L::Shift => 42,
        L::Alt => 56,
        L::Meta => 125,
        L::A => 30,
        L::B => 48,
        L::C => 46,
        L::D => 32,
        L::E => 18,
        L::F => 33,
        L::G => 34,
        L::H => 35,
        L::I => 23,
        L::J => 36,
        L::K => 37,
        L::L => 38,
        L::M => 50,
        L::N => 49,
        L::O => 24,
        L::P => 25,
        L::Q => 16,
        L::R => 19,
        L::S => 31,
        L::T => 20,
        L::U => 22,
        L::V => 47,
        L::W => 17,
        L::X => 45,
        L::Y => 21,
        L::Z => 44,
        L::Digit0 => 11,
        L::Digit1 => 2,
        L::Digit2 => 3,
        L::Digit3 => 4,
        L::Digit4 => 5,
        L::Digit5 => 6,
        L::Digit6 => 7,
        L::Digit7 => 8,
        L::Digit8 => 9,
        L::Digit9 => 10,
        L::Semicolon => 39,
        L::Equal => 13,
        L::Comma => 51,
        L::Minus => 12,
        L::Period => 52,
        L::Slash => 53,
        L::BracketL => 26,
        L::BracketR => 27,
        L::Quote => 40,
        L::Backslash => 43,
        L::Enter => 28,
        L::Escape => 1,
        L::Backspace => 14,
        L::Tab => 15,
        L::Space => 57,
        L::Up => 103,
        L::Down => 108,
        L::Left => 105,
        L::Right => 106,
        L::PageUp => 104,
        L::PageDown => 109,
        L::Home => 102,
        L::End => 107,
        L::Insert => 110,
        L::Delete => 111,
        L::F1 => 59,
        L::F2 => 60,
        L::F3 => 61,
        L::F4 => 62,
        L::F5 => 63,
        L::F6 => 64,
        L::F7 => 65,
        L::F8 => 66,
        L::F9 => 67,
        L::F10 => 68,
        L::F11 => 87,
        L::F12 => 88,
    };
    Some(code)
}

/// 主显示器像素尺寸（denormalize 基准）。Linux uinput 用归一化坐标注入，
/// 此值仅作窗口显示对齐参考；无显示服务时回退 1920×1080。
pub fn primary_screen_size() -> (u32, u32) {
    (1920, 1080)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp::keycodes::Lkc as L;

    #[test]
    fn evdev_map_covers_ranges() {
        assert_eq!(lkc_to_evdev(L::A.as_u32()), Some(30));
        assert_eq!(lkc_to_evdev(L::Z.as_u32()), Some(44));
        assert_eq!(lkc_to_evdev(L::Digit1.as_u32()), Some(2));
        assert_eq!(lkc_to_evdev(L::F12.as_u32()), Some(88));
        assert_eq!(lkc_to_evdev(L::Space.as_u32()), Some(57));
        assert_eq!(lkc_to_evdev(0x70), None);
    }

    #[test]
    fn ioctl_codes_match_uinput_h() {
        assert_eq!(UI_SET_EVBIT, 0x40045564);
        assert_eq!(UI_SET_KEYBIT, 0x40045565);
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_DESTROY, 0x5502);
    }
}
