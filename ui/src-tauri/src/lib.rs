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

/// 共享状态（`tauri::State`）：状态栏快照（FR-09.3）+ 设备在线表（P1.1 下沉，FR-01.2/01.4）
/// + 日志环形缓冲（P2.7，矩阵 §7.5，喂日志页查询/清空）
/// + 会话管理器（P2 遗留 ①，矩阵 §7.5，喂会话建立/键鼠/剪贴板/断开命令）。
///
/// `devices` 用 `Arc<Mutex<DeviceTable>>`：后台任务驱动 mDNS 监听器并把在线表快照镜像进来，
/// `device_list` 命令（无入参）读它出剪枝+排序行——对齐 `apps/ui-server` 的 `/api/devices`。
/// `ring` 用 `Arc<Mutex<LogRing>>`：`tracing` 捕获层写入、`log_query`/`log_clear` 命令读写。
/// `session` 用 `Arc<tokio::sync::Mutex<SessionManager>>`：跨 `#[tauri::command]` 异步
/// 命令串行化 connect/send_control/close（QUIC 会话是异步 + 独占的）。
#[cfg(feature = "tauri")]
pub mod state {
    use landesk_ui::{DeviceTable, LogRing, SessionManager, StatusBarModel};

    pub struct AppState {
        pub status: std::sync::Mutex<StatusBarModel>,
        pub devices: std::sync::Arc<std::sync::Mutex<DeviceTable>>,
        pub ring: std::sync::Arc<std::sync::Mutex<LogRing>>,
        pub session: std::sync::Arc<tokio::sync::Mutex<SessionManager>>,
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

/// 本机信息（P2.7「本机信息 API」，矩阵 §7.5）：出 [`landesk_ui::LocalInfo`] 喂设备页左栏「本机卡」。
/// 设备码/临时密码由宿主态派生（`device_code_from` 稳定摘要 + `gen_temp_password`），
/// `mdns_advertising`/`online` 读壳内共享设备表是否非空（有在线设备 = 本机可达）。
#[cfg(feature = "tauri")]
#[tauri::command]
fn local_info(state: tauri::State<'_, state::AppState>) -> Result<landesk_ui::LocalInfo, landesk_ui::StoreError> {
    let store = landesk_ui::ConfigStore::new();
    let cfg = store.load_or_init()?;
    let device_code = landesk_ui::device_code_from(&cfg.device_name);
    let temp_password = landesk_ui::gen_temp_password();
    // 编码能力未接数据面探测，保守空值；mDNS/在线态由设备表驱动。
    let mut devices = state.devices.lock().unwrap();
    let online = !devices.rows(landesk_ui::now_secs()).is_empty();
    let mdns_advertising = online;
    let info = commands::local_info(
        &store,
        &device_code,
        &temp_password,
        vec![],
        mdns_advertising,
        online,
    )?;
    Ok(info)
}

/// 日志查询（P2.7「日志查询 API」，矩阵 §7.5）：环形缓冲 + 级别/关键字过滤（最旧→最新）。
/// `level`/`keyword` 可空（None = 不限），对齐前端 `LogFilter` / 关键字语义。
#[cfg(feature = "tauri")]
#[tauri::command]
fn log_query(
    state: tauri::State<'_, state::AppState>,
    level: Option<String>,
    keyword: Option<String>,
) -> Vec<landesk_ui::LogLine> {
    let level = level.unwrap_or_else(|| "ALL".into());
    let keyword = keyword.unwrap_or_default();
    commands::log_query(&state.ring.lock().unwrap(), &level, &keyword)
}

/// 日志清空（P2.7「日志清空 API」，矩阵 §7.5）：计数归零，容量不变。
#[cfg(feature = "tauri")]
#[tauri::command]
fn log_clear(state: tauri::State<'_, state::AppState>) {
    commands::log_clear(&mut state.ring.lock().unwrap());
}

/// 会话建立（P2 遗留 ①，矩阵 §7.5「会话建立/鉴权」，FR-01 一键连接）：
/// 设备ID 经共享 `DeviceTable` 解析 `ip:port`，QUIC 拨号 + bootstrap 隧道 + ECDH 会话升级。
/// 出参 [`landesk_ui::SessionResp`]（与 `apps/ui-server` REST 同形 P1.3）：成功带 `status`
/// （新会话快照），设备不在线 / 密码错带 `error`（前端读 `error` 区分，不抛）。
///
/// Tauri v2 约束：**异步命令含 `State`（引用）入参必须返回 `Result`**——故包 `Result<_, String>`；
/// 业务错误已带进 `SessionResp.error`（P1.3 同形），所有分支均 `Ok(resp)`，`Err` 不会触发。
#[cfg(feature = "tauri")]
#[tauri::command]
async fn session_connect(
    state: tauri::State<'_, state::AppState>,
    device_id: String,
    password: String,
    sni: Option<String>,
) -> Result<landesk_ui::SessionResp, String> {
    // 先取设备（std Mutex 非跨 await 持有）；未命中 → 无路径。
    let dev: Option<landesk_ui::OnlineDevice> = {
        let mut devices = state.devices.lock().unwrap();
        devices.find_online(&device_id, landesk_ui::now_secs())
    };
    let sni = sni.unwrap_or_else(|| "localhost".into());
    let mut mgr = state.session.lock().await;
    let resp = match &dev {
        Some(d) => match mgr.connect(d, &password, &sni).await {
            Ok(v) => landesk_ui::SessionResp::ok(v),
            Err(e) => {
                tracing::warn!(%device_id, error = %e, "会话建立失败");
                landesk_ui::SessionResp::fail(Some(mgr.status_view()), e.to_string())
            }
        },
        None => {
            tracing::warn!(%device_id, "目标设备不在线（未发现 / 已过期）");
            landesk_ui::SessionResp::fail(Some(mgr.status_view()), "device not online")
        }
    };
    Ok(resp)
}

/// 发送控制帧（P2 遗留 ①，矩阵 §7.5「键鼠 / 剪贴板 / 截图 / 显示器」，FR-06/FR-07）：
/// 一条 [`landesk_ui::ControlAction`] 意图展开成 1..N 线层帧逐帧发出（剪贴板含 32KB 分段 + SHA-256）。
/// 出参 [`landesk_ui::SessionResp`]：成功带 `sent`（发出帧数）+ `status`；无会话 / 展开超限带 `error`。
/// 同 Tauri 异步+`State` 约束，返回 `Result<_, String>`（错误已带进 `SessionResp`，分支均 `Ok`）。
#[cfg(feature = "tauri")]
#[tauri::command]
async fn session_send_control(
    state: tauri::State<'_, state::AppState>,
    action: landesk_ui::ControlAction,
) -> Result<landesk_ui::SessionResp, String> {
    let mut mgr = state.session.lock().await;
    let resp = match mgr.send_control(&action).await {
        Ok(sent) => {
            let status = mgr.status_view();
            landesk_ui::SessionResp::ok_sent(status, sent)
        }
        Err(e) => {
            tracing::warn!(error = %e, "发送控制帧失败");
            landesk_ui::SessionResp::fail(Some(mgr.status_view()), e.to_string())
        }
    };
    Ok(resp)
}

/// 会话状态（P2 遗留 ①，FR-09.3）：出当前状态机 + 折叠快照（未连 = `idle`）。
/// 同 Tauri 异步+`State` 约束，返回 `Result<_, String>`（无业务错误，恒 `Ok`）。
#[cfg(feature = "tauri")]
#[tauri::command]
async fn session_status(state: tauri::State<'_, state::AppState>) -> Result<landesk_ui::SessionResp, String> {
    let mgr = state.session.lock().await;
    Ok(landesk_ui::SessionResp::ok(mgr.status_view()))
}

/// 会话关闭（P2 遗留 ①，FR-02.4）：优雅断开 QUIC + 状态机落 `Closed`。
/// 同 Tauri 异步+`State` 约束，返回 `Result<_, String>`（恒 `Ok`）。
#[cfg(feature = "tauri")]
#[tauri::command]
async fn session_close(state: tauri::State<'_, state::AppState>) -> Result<landesk_ui::SessionResp, String> {
    let mut mgr = state.session.lock().await;
    mgr.close();
    Ok(landesk_ui::SessionResp::ok(mgr.status_view()))
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

    // 共享日志环形缓冲（P2.7）：捕获层写入、`log_query`/`log_clear` 命令读写。
    let ring: std::sync::Arc<std::sync::Mutex<landesk_ui::LogRing>> =
        Arc::new(std::sync::Mutex::new(landesk_ui::LogRing::new()));

    // 会话管理器（P2 遗留 ①）：信任任意证书的 QUIC 客户端（LAN 内网信任域，FR-01），
    // `insecure_default` 自动装 quinn 内置 ring provider（壳不必直接依赖 quinn/ring）。
    // 跨异步 `#[tauri::command]` 用 tokio Mutex 串行化。
    let session: std::sync::Arc<tokio::sync::Mutex<landesk_ui::SessionManager>> =
        Arc::new(tokio::sync::Mutex::new(
            landesk_ui::SessionManager::new(
                landesk_ui::QuicClient::insecure_default(
                    "0.0.0.0:0".parse().expect("0.0.0.0:0"),
                )
                .expect("quic client init"),
            ),
        ));

    // 最小 `tracing` 订阅器：把事件捕获进 `ring`（离线安全——只依赖壳 lock 里已传的
    // `tracing`/`tracing-core`，不引 `tracing-subscriber`/新 crate，守壳离线门纯净）。
    // 终端无输出（GUI 壳日志进环形缓冲供日志页查询，不刷 stdout 污染 Tauri 窗体）。
    install_ring_subscriber(Arc::clone(&ring));

    tauri::Builder::default()
        .manage(AppState {
            status: std::sync::Mutex::new(landesk_ui::StatusBarModel::new()),
            devices,
            ring,
            session,
        })
        .invoke_handler(tauri::generate_handler![
            config_load_or_init,
            config_save,
            device_list,
            reconnect_view,
            reconnect_schedule,
            choose_path,
            local_info,
            log_query,
            log_clear,
            session_connect,
            session_send_control,
            session_status,
            session_close
        ])
        .run(tauri::generate_context!())
        .expect("tauri app 启动");
}

/// 安装一个最小 [`tracing::Subscriber`]：每条事件提取 `level`/`target`/`message` +
/// 键值串，写进共享 [`landesk_ui::LogRing`]（对齐 `apps/ui-server` 的 `RingLayer`，但
/// **不依赖 `tracing-subscriber`**——壳 vendor 无该 crate，手搓 `Subscriber` 保离线门纯净）。
///
/// span 方法（`new_span`/`record`/`enter`/`exit`）空实现；只在 `event` 里捕获事件。
#[cfg(feature = "tauri")]
fn install_ring_subscriber(ring: std::sync::Arc<std::sync::Mutex<landesk_ui::LogRing>>) {
    struct RingSubscriber {
        ring: std::sync::Arc<std::sync::Mutex<landesk_ui::LogRing>>,
    }

    impl tracing::Subscriber for RingSubscriber {
        /// 全收（环形缓冲自身限容 `DEFAULT_LOG_CAPACITY`，前端查询再过滤）。
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(0x1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let meta = event.metadata();
            let mut kv = RingVisitor::default();
            event.record(&mut kv);
            let line = landesk_ui::LogLine::new(
                meta.level().as_str(),
                meta.target(),
                &kv.message,
                &kv.kv,
                landesk_ui::LogLine::now_micros(),
            );
            self.ring.lock().unwrap().push(line);
        }
    }

    /// 提取 [`tracing::Event`] 字段的 `field::Visit`：`message` 作正文，其余拼 `k=v`。
    #[derive(Default)]
    struct RingVisitor {
        message: String,
        kv: String,
    }

    impl tracing::field::Visit for RingVisitor {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                self.message = value.to_string();
            } else if self.kv.is_empty() {
                self.kv = format!("{}={value}", field.name());
            } else {
                self.kv.push_str(&format!(" {}={value}", field.name()));
            }
        }
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{value:?}");
            } else if self.kv.is_empty() {
                self.kv = format!("{}={value:?}", field.name());
            } else {
                self.kv.push_str(&format!(" {}={value:?}", field.name()));
            }
        }
    }

    let sub = RingSubscriber { ring };
    // `set_default` 失败（Tauri 自身已装全局订阅器）不致命——保留默认，日志页空即可。
    let _ = tracing::subscriber::set_default(sub);
}
