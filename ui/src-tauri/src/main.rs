//! `landesk-app` 可执行入口（形态 B：Tauri GUI 壳）。
//!
//! - feature 关（默认 / 离线）：仅校验纯命令内核可用（无 GUI），打印用法后退出；
//! - feature `tauri` 开：启动 Tauri app（需 GUI 系统依赖）。

// Windows GUI release 版隐藏附带的黑色控制台窗口（ws2 链接子系统选择）：
// - 仅 windows + tauri feature（GUI 形态）+ release（debug 保留控制台便于看 panic）；
// - 纯内核 CLI 模式（无 tauri feature）保留控制台，否则 eprintln 用法看不到。
#![cfg_attr(
    all(windows, feature = "tauri", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    #[cfg(feature = "tauri")]
    landesk_app::run();

    #[cfg(not(feature = "tauri"))]
    {
        eprintln!("landesk-app（纯内核模式，未编译 Tauri 本体）");
        eprintln!("  Tauri GUI 需 Tauri 工具链 + GUI 系统依赖：cargo run --features tauri");
        eprintln!("  纯命令内核（config/devices/status/reconnect/path）可离线验证，见 landesk-ui。");
    }
}
