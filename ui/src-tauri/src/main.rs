//! `landesk-app` 可执行入口（形态 B：Tauri GUI 壳）。
//!
//! - feature 关（默认 / 离线）：仅校验纯命令内核可用（无 GUI），打印用法后退出；
//! - feature `tauri` 开：启动 Tauri app（需 GUI 系统依赖）。

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
