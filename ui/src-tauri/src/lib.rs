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

/// 共享状态（`tauri::State`）：状态栏快照（FR-09.3）+ 设备在线表（P1.1 下沉，FR-01.2/01.4）。
///
/// `devices` 用 `Arc<Mutex<DeviceTable>>`：后台任务驱动 mDNS 监听器并把在线表快照镜像进来，
/// `device_list` 命令（无入参）读它出剪枝+排序行——对齐 `apps/ui-server` 的 `/api/devices`。
#[cfg(feature = "tauri")]
pub mod state {
    use landesk_ui::{DeviceTable, StatusBarModel};

    pub struct AppState {
        pub status: std::sync::Mutex<StatusBarModel>,
        pub devices: std::sync::Arc<std::sync::Mutex<DeviceTable>>,
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

/// 设备列表（FR-01.2/01.4，P1.1 下沉）：**无入参**，读壳内共享 `DeviceTable`
/// （后台 mDNS 监听器驱动 + 镜像在线表），出剪枝+排序行。对齐 `apps/ui-server`
/// 的 `/api/devices`——前端不再需要自己传 `[]`，设备表由壳维护。
#[cfg(feature = "tauri")]
#[tauri::command]
fn device_list(state: tauri::State<'_, state::AppState>) -> Vec<landesk_ui::DeviceRow> {
    state.devices.lock().unwrap().rows(landesk_ui::now_secs())
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

/// 重连退避计划（P1.2「会话命令面」，FR-05.4）：出完整 1/2/4/8/16s 序列 + 次数，
/// 喂 `Session.vue` 重连浮层（替代前端硬编码 `2/5` 与退避文案）。
#[cfg(feature = "tauri")]
#[tauri::command]
fn reconnect_schedule() -> landesk_ui::ReconnectSchedule {
    commands::reconnect_schedule(&landesk_ui::ReconnectScheduler::default_policy())
}

/// 连接路径决策（FR-05/FR-11）：内网→P2P→中继→NoPath。
/// 出参用双宿主共享的 [`landesk_ui::PathDecisionView`]（P1.3 归一：与 REST
/// `/api/choose-path` 出参同形——有路径 `{"connMode":"p2p"}`、无路径 `{"nopath":true}`）。
#[cfg(feature = "tauri")]
#[tauri::command]
fn choose_path(
    inner_found: bool,
    hole: landesk_ui::HoleResult,
    relay_enabled: bool,
) -> landesk_ui::PathDecisionView {
    landesk_ui::PathDecisionView::from(&commands::choose_path(inner_found, &hole, relay_enabled))
}

/// 构建并运行 Tauri app（feature `tauri`；需 Tauri CLI + GUI 系统依赖）。
#[cfg(feature = "tauri")]
pub fn run() {
    use std::sync::Arc;
    use state::AppState;

    // 共享设备在线表（P1.1 下沉）：后台 mDNS 驱动 + `device_list` 命令读它。
    let devices: std::sync::Arc<std::sync::Mutex<landesk_ui::DeviceTable>> =
        Arc::new(std::sync::Mutex::new(landesk_ui::DeviceTable::new()));

    // 启动 mDNS 监听器（设备发现）。失败不致命（无网卡 / headless），降级空表。
    let listener = match landesk_ui::Listener::start() {
        Ok(l) => Some(l),
        Err(e) => {
            eprintln!("landesk-app: mDNS 监听启动失败，设备列表降级为空（{e}）");
            None
        }
    };
    // 克隆 Arc 进后台任务（原句柄留给 state 共享）。
    let devices_bg = Arc::clone(&devices);
    tauri::async_runtime::spawn(async move {
        let mut l = listener;
        loop {
            // `tauri::async_runtime` 只 re-export `spawn`（无 `sleep`）；壳直接依赖
            // `tokio`（full，含 time），且 Tauri v2 运行体即 tokio，故用 `tokio::time::sleep`。
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let snap = match l.as_mut() {
                Some(li) => {
                    while li.poll_next().is_some() {}
                    li.snapshot()
                }
                None => Vec::new(),
            };
            devices_bg.lock().unwrap().apply_snapshot(snap);
        }
    });

    tauri::Builder::default()
        .manage(AppState {
            status: std::sync::Mutex::new(landesk_ui::StatusBarModel::new()),
            devices,
        })
        .invoke_handler(tauri::generate_handler![
            config_load_or_init,
            config_save,
            device_list,
            reconnect_view,
            reconnect_schedule,
            choose_path
        ])
        .run(tauri::generate_context!())
        .expect("tauri app 启动");
}
