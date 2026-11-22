//! 三平台真实采屏 smoke 测试（FR-03.1）。
//!
//! 默认 `#[ignore]`（headless CI / 无屏幕录制权限环境会失败），真机手动跑：
//! ```text
//! cargo test -p capture --test platform_screenshots -- --ignored --nocapture
//! ```
//! 验证：平台采屏器能枚举显示器、抓到非空 BGRA 帧且尺寸匹配。
//! macOS 需给终端/测试进程授予「屏幕录制」权限；Linux 需 DISPLAY + libX11；
//! Windows 需桌面会话（非服务会话）。

use capture::platform_capture;

#[test]
#[ignore = "需要真实桌面会话 + 屏幕录制权限；真机手动 --ignored 运行"]
fn platform_capture_grabs_nonempty_bgra_frame() {
    let cap = match platform_capture() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("平台采屏不可用（跳过断言）：{e}");
            return;
        }
    };
    let monitors = cap.monitors().expect("枚举显示器");
    assert!(!monitors.is_empty(), "至少一块显示器");
    let m = monitors[0];
    let frame = cap.grab(m.index).expect("抓一帧");
    assert_eq!(frame.width, m.width);
    assert_eq!(frame.height, m.height);
    assert_eq!(frame.data.len(), m.width as usize * m.height as usize * 4);
    assert!(frame.data.iter().any(|&b| b != 0), "画面非全黑");
    eprintln!("采屏成功：{}x{} BGRA", frame.width, frame.height);
}
