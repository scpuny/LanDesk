//! 纯命令内核（M8）：Tauri 命令 handler 的**无 GUI 逻辑**，全部调 [`landesk_ui`] 纯函数。
//!
//! 本模块**不 import tauri**——只做「入参 → 纯逻辑 → 可序列化出参」，故可离线单测
//! （feature 关时主仓 `cargo test` 也能跑）。`lib.rs` 里再把它们包成 `#[tauri::command]`。
//!
//! 业务类型全部经 `landesk-ui` 便捷 re-export 取到（壳只依赖内核一个 crate）。

use landesk_ui::{
    Config, ConfigStore, CoreEvent, DeviceRow, HoleResult, OnlineDevice, PathDecision,
    ReconnectScheduler, ReconnectView, StatusBarModel, decide_path, device_rows, view_of,
};

/// 配置页：读或首次生成默认模板（FR-08）。
pub fn config_load_or_init(store: &ConfigStore) -> Result<Config, landesk_ui::StoreError> {
    store.load_or_init()
}

/// 配置页：保存（FR-08）。
pub fn config_save(store: &ConfigStore, c: &Config) -> Result<(), landesk_ui::StoreError> {
    store.save(c)
}

/// 设备列表：从在线设备（含 TTL 剪枝 + 排序）出可序列化行（FR-01.2/01.4）。
pub fn device_list(devices: &mut Vec<OnlineDevice>, now_epoch_secs: u64) -> Vec<DeviceRow> {
    device_rows(devices, now_epoch_secs)
}

/// 状态栏：把一条核心事件折进快照（FR-09.3）。调用方持 `StatusBarModel` 状态。
pub fn apply_status(model: &mut StatusBarModel, ev: &CoreEvent) {
    model.apply(ev);
}

/// 重连指示器：由调度器进度出 UI 视图（FR-05.4，接 M7）。
pub fn reconnect_view(sched: &ReconnectScheduler) -> Option<ReconnectView> {
    view_of(sched)
}

/// 连接路径决策（FR-05/FR-11）：内网→P2P→中继→NoPath。
pub fn choose_path(inner_found: bool, hole: &HoleResult, relay_enabled: bool) -> PathDecision {
    decide_path(inner_found, hole, relay_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use landesk_ui::ConnMode;

    #[test]
    fn path_decision_smoke() {
        assert_eq!(
            choose_path(true, &HoleResult::NoPath, false),
            PathDecision::Use(ConnMode::Inner)
        );
        assert_eq!(choose_path(false, &HoleResult::NoPath, false), PathDecision::NoPath);
        assert_eq!(
            choose_path(false, &HoleResult::NeedRelay, true),
            PathDecision::Use(ConnMode::Relay)
        );
    }

    #[test]
    fn config_roundtrip_temp() {
        let path =
            std::env::temp_dir().join(format!("landesk-cmd-{}.json", std::process::id()));
        let store = ConfigStore::with_path(&path);
        let c = config_load_or_init(&store).expect("init");
        let mut c2 = c.clone();
        c2.device_name = "test-dev".into();
        config_save(&store, &c2).expect("save");
        assert_eq!(
            config_load_or_init(&store).expect("reload").device_name,
            "test-dev"
        );
        let _ = std::fs::remove_file(&path);
    }
}
