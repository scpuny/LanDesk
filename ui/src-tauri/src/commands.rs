//! 纯命令内核（M8）：Tauri 命令 handler 的**无 GUI 逻辑**，全部调 [`landesk_ui`] 纯函数。
//!
//! 本模块**不 import tauri**——只做「入参 → 纯逻辑 → 可序列化出参」，故可离线单测
//! （feature 关时主仓 `cargo test` 也能跑）。`lib.rs` 里再把它们包成 `#[tauri::command]`。
//!
//! 业务类型全部经 `landesk-ui` 便捷 re-export 取到（壳只依赖内核一个 crate）。

use landesk_ui::{
    Config, ConfigStore, CoreEvent, DeviceRow, HoleResult, LocalInfo, LogLine, LogRing,
    OnlineDevice, PathDecision, ReconnectSchedule, ReconnectScheduler, ReconnectView,
    StatusBarModel, decide_path, device_rows, local_info_of, schedule_of, view_of,
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

/// 重连退避计划（P1.2「会话命令面」，FR-05.4）：出完整 1/2/4/8/16 计划，
/// 喂 `Session.vue` 重连浮层替代硬编码 `2/5`。
pub fn reconnect_schedule(sched: &ReconnectScheduler) -> ReconnectSchedule {
    schedule_of(sched)
}

/// 连接路径决策（FR-05/FR-11）：内网→P2P→中继→NoPath。
pub fn choose_path(inner_found: bool, hole: &HoleResult, relay_enabled: bool) -> PathDecision {
    decide_path(inner_found, hole, relay_enabled)
}

/// 本机信息（P2.7「本机信息 API」，矩阵 §7.5）：由配置 + 宿主态出 [`LocalInfo`]。
/// 宿主传 `device_code`/`temp_password`（通常 [`landesk_ui::device_code_from`] /
/// [`landesk_ui::gen_temp_password`] 派生）、`enc` 能力、`mdns_advertising`/`online` 宿主态。
pub fn local_info(
    store: &ConfigStore,
    device_code: &str,
    temp_password: &str,
    enc: Vec<String>,
    mdns_advertising: bool,
    online: bool,
) -> Result<LocalInfo, landesk_ui::StoreError> {
    let cfg = store.load_or_init()?;
    Ok(local_info_of(&cfg, device_code, temp_password, enc, mdns_advertising, online))
}

/// 日志查询（P2.7「日志查询 API」，矩阵 §7.5）：环形缓冲 + 级别/关键字过滤（最旧→最新）。
pub fn log_query(ring: &LogRing, level: &str, keyword: &str) -> Vec<LogLine> {
    ring.export_lines(level, keyword)
}

/// 日志清空（P2.7「日志清空 API」，矩阵 §7.5）：计数归零，容量不变。
pub fn log_clear(ring: &mut LogRing) {
    ring.clear();
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

    #[test]
    fn local_info_kernel() {
        let path =
            std::env::temp_dir().join(format!("landesk-cmd-li-{}.json", std::process::id()));
        let store = ConfigStore::with_path(&path);
        // 首次读生成默认模板（device_name 空），local_info 据此派生。
        let info = local_info(&store, "83FA217C", "629418", vec!["QSV".into()], true, true)
            .expect("local_info");
        assert_eq!(info.device_code, "83FA217C");
        assert_eq!(info.temp_password, "629418");
        assert_eq!(info.enc, vec!["QSV".to_string()]);
        assert!(info.mdns, "mdns_enabled(默认 true) + 广播中 → 真");
        assert!(info.online);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_query_and_clear() {
        let mut ring = LogRing::new();
        ring.push(LogLine::new("INFO", "session", "connected", "peer=ABCD", 1));
        ring.push(LogLine::new("ERROR", "auth", "failed", "peer=ABCD", 2));
        assert_eq!(log_query(&ring, "ALL", "").len(), 2, "全级别 2 条");
        assert_eq!(log_query(&ring, "ERROR", "").len(), 1, "仅 ERROR");
        assert_eq!(log_query(&ring, "ALL", "auth").len(), 1, "关键字命中");
        log_clear(&mut ring);
        assert!(ring.is_empty(), "清空后空");
    }
}
