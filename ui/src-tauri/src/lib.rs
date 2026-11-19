//! `landesk-app` — LanDesk Tauri 壳（形态 B，M8，对齐 `docs/architecture.md` §3 `ui/src-tauri`）。
//!
//! 双轨设计（离线友好）：
//! - **feature 关（默认）**：只有 [`commands`] 纯内核（调 [`landesk_ui`]，不 import tauri），
//!   可离线编译 + 单测（主仓 `cargo test -p landesk-ui` 间接覆盖其逻辑）；
//! - **feature `tauri` 开**：编译 Tauri app 本体（`tauri` + `tauri-build`，需 GUI/系统依赖，
//!   离线 vendor 不含）——[`run`] + `#[tauri::command]` 把内核包成前端可调命令。
//!
//! 前端（`ui/frontend`，Vue3 + Arco）经 `lib/landesk.js` 调这些命令；命令出参全部可
//! `serde` 序列化（`landesk-ui` 模型已 `Serialize`）。

pub mod commands;

// ---------- Tauri 本体（仅 feature `tauri` 时编译；离线 vendor 不含 tauri）----------

/// 共享状态（`tauri::State`）：状态栏快照（喂前端 StatusBar，FR-09.3）。
#[cfg(feature = "tauri")]
pub mod state {
    use landesk_ui::StatusBarModel;
    use std::sync::Mutex;

    pub struct AppState {
        pub status: Mutex<StatusBarModel>,
    }
}

/// 配置页：读或首次生成默认模板（FR-08）。
#[cfg(feature = "tauri")]
#[tauri::command]
fn config_load_or_init() -> Result<landesk_ui::Config, landesk_ui::StoreError> {
    let store = landesk_ui::ConfigStore::new();
    commands::config_load_or_init(&store)
}

/// 配置页：保存（FR-08）。
#[cfg(feature = "tauri")]
#[tauri::command]
fn config_save(c: landesk_ui::Config) -> Result<(), landesk_ui::StoreError> {
    let store = landesk_ui::ConfigStore::new();
    commands::config_save(&store, &c)
}

/// 设备列表（FR-01.2/01.4）：入参在线设备 + 当前 epoch 秒，出参可序列化行。
#[cfg(feature = "tauri")]
#[tauri::command]
fn device_list(
    mut devices: Vec<landesk_ui::OnlineDevice>,
    now_epoch_secs: u64,
) -> Vec<landesk_ui::DeviceRow> {
    commands::device_list(&mut devices, now_epoch_secs)
}

/// 重连指示器（FR-05.4）：按已用尝试数出「下次」视图。
#[cfg(feature = "tauri")]
#[tauri::command]
fn reconnect_view(used_attempts: u32) -> Option<landesk_ui::ReconnectView> {
    let mut s = landesk_ui::ReconnectScheduler::default_policy();
    for _ in 0..used_attempts {
        s.step();
    }
    commands::reconnect_view(&s)
}

/// 连接路径决策（FR-05/FR-11）：内网→P2P→中继→NoPath。
#[cfg(feature = "tauri")]
#[tauri::command]
fn choose_path(
    inner_found: bool,
    hole: landesk_ui::HoleResult,
    relay_enabled: bool,
) -> landesk_ui::PathDecision {
    commands::choose_path(inner_found, &hole, relay_enabled)
}

/// 构建并运行 Tauri app（feature `tauri`；需 Tauri CLI + GUI 系统依赖）。
#[cfg(feature = "tauri")]
pub fn run() {
    use state::AppState;
    tauri::Builder::default()
        .manage(AppState {
            status: std::sync::Mutex::new(landesk_ui::StatusBarModel::new()),
        })
        .invoke_handler(tauri::generate_handler![
            config_load_or_init,
            config_save,
            device_list,
            reconnect_view,
            choose_path
        ])
        .run(tauri::generate_context!())
        .expect("tauri app 启动");
}
