//! 统一 LanDesk Key Code（LKC，见 `docs/protocol.md` §1.4）。
//!
//! 跨平台键码映射在 `input` crate 按平台实现；本模块只定义编码值与
//! 修饰键位常量，保持协议层平台无关。

/// 修饰键位（与 [`crate::msg::MouseMove::mods`] 一致）。
pub const MOD_CTRL: u8 = 0b0001;
pub const MOD_SHIFT: u8 = 0b0010;
pub const MOD_ALT: u8 = 0b0100;
pub const MOD_META: u8 = 0b1000;

/// LKC 值域：0–255。各键段严格不重叠（未分配区段预留功能键扩展）。
///
/// 分段布局：
/// ```text
/// 0x01-0x04 修饰键    0x10-0x29 字母 A-Z   0x30-0x39 数字 0-9
/// 0x40-0x49 常用符号  0x50-0x54 编辑键     0x58-0x6D 方向键/滚动/F1-F12
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Lkc {
    // 修饰键 0x01-0x04（与 mods 位冗余，便于单键事件）
    Ctrl = 0x01,
    Shift = 0x02,
    Alt = 0x03,
    Meta = 0x04,

    // 字母 A-Z：0x10-0x29
    A = 0x10,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,

    // 数字 0-9：0x30-0x39
    Digit0 = 0x30,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    // 常用符号：0x40-0x49
    Semicolon = 0x40,
    Equal,
    Comma,
    Minus,
    Period,
    Slash,
    BracketL,
    BracketR,
    Quote,
    Backslash,

    // 编辑键：0x50-0x54
    Enter = 0x50,
    Escape,
    Backspace,
    Tab,
    Space,

    // 方向键：0x58-0x5B
    Up = 0x58,
    Down,
    Left,
    Right,

    // 编辑/滚动：0x5C-0x61
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,

    // F 键：0x62-0x6D
    F1 = 0x62,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl Lkc {
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// 未知/未映射 LKC 值。
    pub fn from_u32(v: u32) -> Option<Self> {
        // 先校验值是否落在某个已定义键段内，再用 transmute 构造
        // （各键段判别值连续且互不重叠，byte 在段内即可安全还原为枚举变体）。
        let byte = u8::try_from(v).ok()?;
        const KNOWN: &[(u8, u8)] = &[
            (0x01, 0x04), // 修饰键
            (0x10, 0x29), // 字母 A-Z
            (0x30, 0x39), // 数字 0-9
            (0x40, 0x49), // 常用符号
            (0x50, 0x54), // 编辑键
            (0x58, 0x6d), // 方向键 / 滚动 / F1-F12
        ];
        if !KNOWN.iter().any(|(lo, hi)| byte >= *lo && byte <= *hi) {
            return None;
        }
        unsafe { Some(std::mem::transmute::<u8, Lkc>(byte)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_common_keys() {
        // 各键段边界值：修饰键 / 字母首尾 / 数字首尾 / 符号 / 编辑键 / 方向键 / F 键
        for v in [
            0x01u32, // Ctrl
            0x10,    // A
            0x29,    // Z
            0x30,    // Digit0
            0x39,    // Digit9
            0x40,    // Semicolon
            0x50,    // Enter
            0x58,    // Up
            0x62,    // F1
            0x6d,    // F12
        ] {
            let k = Lkc::from_u32(v).expect("known key");
            assert_eq!(k.as_u32(), v);
        }
    }

    #[test]
    fn unknown_key_returns_none() {
        // 0x05（修饰键段之后）与 0x70（F12 之后）均未分配
        assert!(Lkc::from_u32(0x05).is_none());
        assert!(Lkc::from_u32(0x70).is_none());
        // 0x55/0x56/0x57 是编辑键段与方向键段之间的保留空隙
        assert!(Lkc::from_u32(0x57).is_none());
    }

    #[test]
    fn letter_and_digit_do_not_collide() {
        // 字母 Q 位于 0x10+16=0x20，数字 0 位于 0x30，二者不得同值
        assert_ne!(Lkc::Q.as_u32(), Lkc::Digit0.as_u32());
        assert_eq!(Lkc::Q.as_u32(), 0x20);
        assert_eq!(Lkc::Digit0.as_u32(), 0x30);
    }
}
