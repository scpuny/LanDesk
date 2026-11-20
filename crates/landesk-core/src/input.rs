//! M5 输入注入全链路（FR-06，对齐 `docs/architecture.md` §3 landesk-core `control.rs` 的键盘/鼠标分支）。
//!
//! 控制流上的 [`ControlMsg`]（鼠标 / 键盘）在受控端落地为 [`input::InputInjector`]
//! 调用：坐标按归一化值透传（平台实现内 `denormalize` 还原），按键经
//! [`input::is_supported_lkc`] 校验，未知键码跳过不报错（协议演进预留）。
//!
//! 本模块纯逻辑 + trait 对象，可离线单测（用记录型 mock 注入器）。

use input::{InputError, InputInjector, MouseButtonKind};
use lp::msg::ControlMsg;

/// 一条输入帧的分发结果（供受控端事件循环 / 日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    /// 已注入平台。
    Injected,
    /// 未知 LKC 键码，跳过（不视为错误，协议演进预留）。
    SkippedUnknownKey(u32),
    /// 未知鼠标按键编码，跳过。
    SkippedUnknownButton(u8),
}

/// 把一条控制流输入帧映射到 [`InputInjector`]（受控端「键鼠可用」核心链路）。
///
/// 仅处理鼠标 / 键盘帧；其它控制帧（剪贴板 / 心跳 / 参数…）返回 [`DispatchResult::Injected`]
/// 的哨兵语义不适用——调用方应先按 `msg_type` 分流，本函数只负责输入类帧。
///
/// - `screen_w` / `screen_h`：受控端屏幕分辨率（供归一化坐标还原，注入器内部 `denormalize`）。
/// - `msg`：来自控制端的输入帧（见 [`lp::msg::MouseMove`] 等）。
pub fn apply_input(
    injector: &dyn InputInjector,
    msg: &ControlMsg,
    screen_w: u32,
    screen_h: u32,
) -> Result<DispatchResult, InputError> {
    match msg {
        ControlMsg::MouseMove(m) => {
            injector.mouse_move(m.x, m.y, screen_w, screen_h)?;
            Ok(DispatchResult::Injected)
        }
        ControlMsg::MouseButton(b) => match mouse_button_kind(b.button) {
            Some(kind) => {
                injector.mouse_button(kind, b.down != 0)?;
                Ok(DispatchResult::Injected)
            }
            None => Ok(DispatchResult::SkippedUnknownButton(b.button)),
        },
        ControlMsg::MouseWheel(w) => {
            injector.mouse_wheel(w.dx, w.dy)?;
            Ok(DispatchResult::Injected)
        }
        ControlMsg::KeyDown(k) => Ok(apply_key(injector, k, true)?),
        ControlMsg::KeyUp(k) => Ok(apply_key(injector, k, false)?),
        _ => Ok(DispatchResult::Injected),
    }
}

/// 按键注入（down/up 共用 LKC 校验 + 注入器调用）。
fn apply_key(
    injector: &dyn InputInjector,
    k: &lp::msg::KeyEvent,
    down: bool,
) -> Result<DispatchResult, InputError> {
    if !input::is_supported_lkc(k.key) {
        return Ok(DispatchResult::SkippedUnknownKey(k.key));
    }
    if down {
        injector.key_down(k.key, k.mods)?;
    } else {
        injector.key_up(k.key, k.mods)?;
    }
    Ok(DispatchResult::Injected)
}

/// 鼠标按键编码（协议 §1.3：0=左 1=右 2=中）→ [`MouseButtonKind`]。
fn mouse_button_kind(button: u8) -> Option<MouseButtonKind> {
    match button {
        0 => Some(MouseButtonKind::Left),
        1 => Some(MouseButtonKind::Right),
        2 => Some(MouseButtonKind::Middle),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use lp::msg::{KeyEvent, MouseButton, MouseMove, MouseWheel};

    /// 记录型注入器：把收到的调用标签记进共享 `Vec`（trait 方法为 `&self`，
    /// 故用 `Arc<Mutex<..>>` 提供内层可变性），断言分发正确。
    #[derive(Default)]
    struct Recorder {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn log(&self, tag: &str) {
            self.calls.lock().unwrap().push(tag.to_owned());
        }
        fn recorded(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl InputInjector for Recorder {
        fn mouse_move(&self, _x: u16, _y: u16, _w: u32, _h: u32) -> Result<(), InputError> {
            self.log("move");
            Ok(())
        }
        fn mouse_button(&self, _button: MouseButtonKind, _down: bool) -> Result<(), InputError> {
            self.log("button");
            Ok(())
        }
        fn mouse_wheel(&self, _dx: i16, _dy: i16) -> Result<(), InputError> {
            self.log("wheel");
            Ok(())
        }
        fn key_down(&self, _key: u32, _mods: u8) -> Result<(), InputError> {
            self.log("keydown");
            Ok(())
        }
        fn key_up(&self, _key: u32, _mods: u8) -> Result<(), InputError> {
            self.log("keyup");
            Ok(())
        }
    }

    #[test]
    fn mouse_frames_injected() {
        let r = Recorder::new();
        let mm = ControlMsg::MouseMove(MouseMove {
            x: 32768,
            y: 32768,
            buttons: 0,
            mods: 0,
        });
        assert_eq!(
            apply_input(&r, &mm, 1920, 1080).unwrap(),
            DispatchResult::Injected
        );
        assert_eq!(r.recorded(), vec!["move".to_owned()], "MouseMove 注入 mouse_move");

        let mb = ControlMsg::MouseButton(MouseButton { button: 0, down: 1 });
        assert_eq!(
            apply_input(&r, &mb, 1920, 1080).unwrap(),
            DispatchResult::Injected,
            "左键应注入"
        );
        let bad = ControlMsg::MouseButton(MouseButton { button: 7, down: 1 });
        assert_eq!(
            apply_input(&r, &bad, 1920, 1080).unwrap(),
            DispatchResult::SkippedUnknownButton(7),
            "未知按键编码跳过"
        );

        let mw = ControlMsg::MouseWheel(MouseWheel { dx: 1, dy: -2 });
        assert_eq!(
            apply_input(&r, &mw, 1920, 1080).unwrap(),
            DispatchResult::Injected
        );
        assert_eq!(
            r.recorded(),
            vec!["move".to_owned(), "button".to_owned(), "wheel".to_owned()],
            "仅已知的鼠标帧被注入，未知按键跳过"
        );
    }

    #[test]
    fn key_frames_injected_and_unknown_skipped() {
        let r = Recorder::new();
        // Lkc::A = 0x10（已知），Lkc::F12 = 0x6d（已知）。
        let kd = ControlMsg::KeyDown(KeyEvent { key: 0x10, mods: 0 });
        assert_eq!(
            apply_input(&r, &kd, 1920, 1080).unwrap(),
            DispatchResult::Injected
        );
        let ku = ControlMsg::KeyUp(KeyEvent { key: 0x6d, mods: 0 });
        assert_eq!(
            apply_input(&r, &ku, 1920, 1080).unwrap(),
            DispatchResult::Injected
        );
        assert_eq!(r.recorded(), vec!["keydown".to_owned(), "keyup".to_owned()]);

        // 0x70 在 LKC 值域外 → 跳过不报错、不注入。
        let unknown = ControlMsg::KeyDown(KeyEvent { key: 0x70, mods: 0 });
        assert_eq!(
            apply_input(&r, &unknown, 1920, 1080).unwrap(),
            DispatchResult::SkippedUnknownKey(0x70)
        );
        assert_eq!(r.recorded().len(), 2, "未知键码不产生注入调用");
    }
}

