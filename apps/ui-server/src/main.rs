//! LanDesk 浏览器 UI 服务（形态A 旁路，M8/M9 本地验证用）。
//!
//! Tauri 窗口本体需要联网装 `cargo-tauri` CLI（离线 vendor 不含），无法在本环境编译。
//! 本二进制提供**同等的可运行 UI 入口**：axum 托管 `ui/frontend` 构建产物 + REST API，
//! 浏览器直接打开即完整 UI。全部复用已 vendor 的 crate（`landesk-ui` 命令内核 + `mdns`
//! 监听器 + axum），不引入新依赖、不进 Tauri 门，离线 `cargo build --offline` 可编译。
//!
//! 启动前置：`ui/frontend` 需先 `npm run build` 产出 `ui/frontend/dist`（本二进制不依赖 Node）。
//! 用法：`cargo run -p ui-server`（默认 127.0.0.1:8080，托管 `ui/frontend/dist`）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, Request, State},
    http::{header, Method, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use futures_util::stream;
use     landesk_ui::{
    device_code_from, ControlAction, Config, ConfigStore, ControlledHost, DeviceTable, HoleResult,
    LogLine, LogRing, OnlineDevice, PathDecisionView, ReconnectScheduler, SessionManager,
    SessionResp, VideoFrameMsg, decide_path, gen_temp_password, local_info_of, schedule_of, view_of,
};
use mdns::Listener;
use std::convert::Infallible;
use tracing::{Event as TraceEvent, Subscriber, field};
use tracing_subscriber::{
    Layer, layer::{Context, SubscriberExt}, registry::LookupSpan, util::SubscriberInitExt,
};

/// 默认托管的前端构建产物目录（相对本仓根；可用 `--static-dir` 覆盖）。
fn default_static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("ui/frontend/dist")
}

/// CLI 参数。
#[derive(Parser)]
#[command(name = "ui-server", about = "LanDesk 浏览器 UI 服务（axum 托管前端 + REST API，Rust 直接启动 UI）")]
struct Args {
    /// HTTP 监听地址。
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: SocketAddr,
    /// 托管的前端构建产物目录（`ui/frontend/dist`）。
    #[arg(long)]
    static_dir: Option<PathBuf>,
    /// 设备发现刷新间隔（秒）：后台周期驱动 mDNS 事件并剪枝在线表。
    #[arg(long, default_value_t = 1)]
    discover: u64,
}

/// 共享状态：mDNS 监听器 + 在线设备表 + 配置存储 + 日志环形缓冲 + 会话管理器 + 静态目录。
/// 手动 `Clone`（`Listener` 非 Clone，用 `Arc<Mutex<…>>` 共享；`SessionManager` 是异步会话，
/// 用 `Arc<tokio::sync::Mutex<…>>` 跨 handler 串行化 connect/send_control/close）。
#[derive(Clone)]
struct Ctx {
    listener: Arc<Mutex<Option<Listener>>>,
    devices: Arc<Mutex<DeviceTable>>,
    store: ConfigStore,
    ring: Arc<Mutex<LogRing>>,
    /// 会话管理器（P2 遗留 ①，矩阵 §7.5）：QUIC 客户端 + 活跃会话 + 状态栏折叠。
    session: Arc<tokio::sync::Mutex<SessionManager>>,
    /// GUI 兼作受控端（提醒4）：QUIC 服务端 accept 任务保活（Arc 共享，drop 时 abort）。
    /// 下划线前缀：仅持有保活，无读取点（状态走 `/api/controlled-status` 从配置派生）。
    _controlled: Arc<Option<ControlledHost>>,
    static_dir: PathBuf,
}

/// `/api/choose-path` 入参（字段名对齐前端 REST 的 camelCase，与 Tauri invoke 约定一致）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChoosePathReq {
    inner_found: bool,
    hole: HoleResult,
    relay_enabled: bool,
}

/// `/api/reconnect` 入参（camelCase 对齐前端）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReconnectReq {
    used_attempts: u32,
}

/// `/api/logs` 查询参数（camelCase 对齐前端 `LogFilter` / 关键字）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogQuery {
    /// 级别过滤（`ALL`/`INFO`/`WARN`/`ERROR`/`DEBUG`；缺省 `ALL`）。
    level: Option<String>,
    /// 关键字（过滤 `mod`/`msg`/`kv`，小写匹配；缺省不限）。
    q: Option<String>,
}

/// `/api/session/connect` 入参（P2 遗留 ①，矩阵 §7.5「会话建立」）：设备ID + 访问密码。
///
/// `sni` 缺省 `"localhost"`（环回 / 同机）；跨机传受控端主机名。`device_id` 须经
/// `/api/devices`（共享 [`DeviceTable`]）解析出 `ip:port` 拨号目标（FR-01）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionConnectReq {
    device_id: String,
    password: String,
    #[serde(default)]
    sni: Option<String>,
}

/// `/api/session/control` 入参（矩阵 §7.5「键鼠 / 剪贴板 / 截图 / 显示器」）：一条控制意图。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionControlReq {
    action: ControlAction,
}

// 会话出参统一外壳（P2 遗留 ①，双宿主响应形状归一）：经 `landesk_ui::SessionResp`，
// 与 `ui/src-tauri` 命令出参同形——成功带 `status`/`sent`，失败带 `error`（HTTP 恒 200）。
// （`SessionResp` 定义在 `landesk-ui::manager`，经 `landesk_ui` re-export；此处不再本地重复定义。）

/// 把 `tracing` 事件捕获进共享 [`LogRing`] 的 [`Layer`]（P2「日志查询 API」）。
///
/// 与 `fmt` 层并存（终端仍打印），同时把每条事件写进环形缓冲供 `/api/logs` 查询。
/// `ring` 用 `Arc<Mutex<LogRing>>` 共享给 [`Ctx`]；当前条数用 `LogRing::len` 读。
struct RingLayer {
    ring: Arc<Mutex<LogRing>>,
}

impl RingLayer {
    fn new(ring: Arc<Mutex<LogRing>>) -> Self {
        Self { ring }
    }
}

impl<S> Layer<S> for RingLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    Self: 'static,
{
    fn on_event(&self, event: &TraceEvent, _ctx: Context<S>) {
        let meta = event.metadata();
        let level = meta.level().as_str();
        let target = meta.target();
        // 提取 message 正文 + 其余键值字段（`k=v` 拼接）。
        let mut kv = KvVisitor::default();
        event.record(&mut kv);
        let line = LogLine::new(level, target, &kv.message, &kv.kv, LogLine::now_micros());
        self.ring.lock().unwrap().push(line);
    }
}

/// 提取 [`Event`] 字段的 [`Visit`] 访问器：`message` 字段作正文，其余拼成 `k=v` 串。
///
/// 覆写 `record_str`（字符串值去引号，最干净）+ `record_debug`（兜底，`DisplayValue` 的
/// Debug 委托 Display 故 message 正文不带引号；`&str` 的 Debug 会带引号故单独走 `record_str`）。
#[derive(Default)]
struct KvVisitor {
    message: String,
    kv: String,
}

impl field::Visit for KvVisitor {
    fn record_str(&mut self, field: &field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.kv.is_empty() {
            self.kv = format!("{}={value}", field.name());
        } else {
            self.kv.push_str(&format!(" {}={value}", field.name()));
        }
    }

    fn record_debug(&mut self, field: &field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else if self.kv.is_empty() {
            self.kv = format!("{}={value:?}", field.name());
        } else {
            self.kv.push_str(&format!(" {}={value:?}", field.name()));
        }
    }
}

/// 当前 unix 时间（秒）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // 日志环形缓冲（P2「日志查询 API」）：捕获层与共享 Ctx 各持一份 Arc。
    let ring = Arc::new(Mutex::new(LogRing::new()));

    // tracing：终端 fmt 层（带 env-filter）+ 环形缓冲捕获层并存。
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(RingLayer::new(Arc::clone(&ring)))
        .init();

    let static_dir = args.static_dir.clone().unwrap_or_else(default_static_dir);
    tracing::info!(bind = %args.bind, static_dir = %static_dir.display(), "ui-server 启动");

    // 启动 mDNS 监听（设备发现）。失败不致命（headless 无网卡），降级空表。
    let listener = match Listener::start() {
        Ok(l) => {
            tracing::info!("mDNS 监听已启动（_landesk._tcp.local.）");
            Some(l)
        }
        Err(e) => {
            tracing::warn!(error = %e, "mDNS 监听启动失败，设备列表降级为空");
            None
        }
    };

    // 启动 mDNS 广播器（FR-01.1）：只听不喊会让两台 ui-server 互相发现不了，
    // 故本机也按配置宣告自己。_advertiser 保活到进程结束（drop 即发 goodbye）。
    let advertiser = {
        let store = ConfigStore::new();
        match store.load_or_init() {
            Ok(cfg) if cfg.mdns_enabled => match landesk_ui::start_advertiser_default(&cfg) {
                Ok(a) => {
                    tracing::info!("mDNS 广播已启动（本机宣告 _landesk._tcp.local.）");
                    Some(Arc::new(a))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "mDNS 广播启动失败，本机不被发现");
                    None
                }
            },
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, "配置读取失败，跳过 mDNS 广播");
                None
            }
        }
    };

    // mDNS 广播心跳：每 60s 重宣告，强制对端 listener 刷新 last_seen（TTL=180s，3 次冗余），
    // 避免组播刷新偶发丢失导致本机从对端列表消失。 advertiser Arc 同时保活到进程结束。
    if let Some(adv) = advertiser.clone() {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                if let Err(e) = adv.heartbeat() {
                    tracing::warn!(error = %e, "mDNS 心跳重宣告失败");
                }
            }
        });
    }
    let _advertiser = advertiser; // 保活（drop 即发 goodbye）

    // GUI 兼作受控端（提醒4，期一）：按设置页访问密码起 QUIC 服务端，接受远端键鼠
    // 控制。空密码 → 不启动（纯控制端）；端口占用等失败 → 降级并告警。存进 Ctx 保活。
    let controlled: Arc<Option<ControlledHost>> = Arc::new({
        let store = ConfigStore::new();
        match store.load_or_init() {
            Ok(cfg) => match ControlledHost::start(&cfg) {
                Ok(h) => {
                    if h.is_serving() {
                        tracing::info!(device_code = %h.device_code(), "GUI 受控服务已启动");
                    }
                    Some(h)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "受控服务启动失败，降级为纯控制端");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "配置读取失败，跳过受控服务");
                None
            }
        }
    });

    // 本机设备码：mDNS 会收到自己的宣告，入表时排除，避免"我的设备"列出自己连不上。
    let mut device_table = DeviceTable::new();
    {
        let store = ConfigStore::new();
        if let Ok(cfg) = store.load_or_init() {
            device_table.set_local_code(device_code_from(&cfg.device_name));
        }
    }
    let ctx = Ctx {
        listener: Arc::new(Mutex::new(listener)),
        devices: Arc::new(Mutex::new(device_table)),
        store: ConfigStore::new(),
        ring,
        // 会话管理器（P2 遗留 ①）：信任任意证书的 QUIC 客户端（LAN 内网信任域，FR-01）。
        // `insecure_default` 自动装 quinn 内置 ring provider（宿主不必直接依赖 quinn/ring）。
        session: Arc::new(tokio::sync::Mutex::new(
            SessionManager::new(
                landesk_ui::QuicClient::insecure_default("0.0.0.0:0".parse().expect("0.0.0.0:0"))
                    .expect("quic client init"),
            ),
        )),
        _controlled: controlled,
        static_dir: static_dir.clone(),
    };

    // 后台任务：周期驱动 mDNS 事件 + 把监听器在线表快照镜像进 DeviceTable + TTL 剪枝（FR-01.4）。
    {
        let ctx = ctx.clone();
        let tick = std::time::Duration::from_secs(args.discover.max(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tick).await;
                // 非阻塞取走已到达事件，再镜像监听器内部在线表进共享 DeviceTable。
                let snap = match ctx.listener.lock().unwrap().as_mut() {
                    Some(l) => {
                        while l.poll_next().is_some() {}
                        l.snapshot()
                    }
                    None => Vec::new(),
                };
                ctx.devices.lock().unwrap().apply_snapshot(snap);
            }
        });
    }

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        // 设备列表（FR-01.2）：共享 DeviceTable（后台任务从监听器镜像）→ TTL 剪枝 + 排序行。
        .route(
            "/api/devices",
            get(|State(ctx): State<Ctx>| async move {
                let now = now_secs();
                Json(ctx.devices.lock().unwrap().rows(now))
            }),
        )
        // 配置读（FR-08）。
        .route(
            "/api/config",
            get(|State(ctx): State<Ctx>| async move {
                match ctx.store.load_or_init() {
                    Ok(c) => Json(c).into_response(),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response(),
                }
            }),
        )
        // 配置写（FR-08）。
        .route(
            "/api/config",
            post(|State(ctx): State<Ctx>, Json(c): Json<Config>| async move {
                match ctx.store.save(&c) {
                    Ok(()) => StatusCode::NO_CONTENT.into_response(),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": e.to_string() })),
                    )
                        .into_response(),
                }
            }),
        )
        // 受控服务状态（提醒4）：设置页展示「受控服务：已启用/未启用」+ 设备码。
        .route(
            "/api/controlled-status",
            get(|State(ctx): State<Ctx>| async move {
                let cfg = ctx.store.load_or_init().unwrap_or_default();
                let device_code = device_code_from(&cfg.device_name);
                let serving = cfg
                    .password
                    .as_deref()
                    .map(|p| !p.trim().is_empty())
                    .unwrap_or(false);
                Json(landesk_ui::ControlledStatus {
                    serving,
                    port: landesk_ui::DEFAULT_PORT,
                    device_code,
                })
                .into_response()
            }),
        )
        // 连接路径决策（FR-05/FR-11）。
        .route(
            "/api/choose-path",
            post(|Json(req): Json<ChoosePathReq>| async move {
                let d = decide_path(req.inner_found, &req.hole, req.relay_enabled);
                Json(PathDecisionView::from(&d)).into_response()
            }),
        )
        // 重连指示器（FR-05.4，接 M7 ReconnectScheduler）。
        .route(
            "/api/reconnect",
            post(|Json(req): Json<ReconnectReq>| async move {
                let mut s = ReconnectScheduler::default_policy();
                for _ in 0..req.used_attempts {
                    s.step();
                }
                match view_of(&s) {
                    Some(v) => Json(v).into_response(),
                    None => StatusCode::NO_CONTENT.into_response(),
                }
            }),
        )
        // 重连退避计划（P1.2「会话命令面」，FR-05.4）：完整 1/2/4/8/16s 序列 + 次数。
        .route(
            "/api/reconnect-schedule",
            get(|| async move {
                Json(schedule_of(&ReconnectScheduler::default_policy())).into_response()
            }),
        )
        // 本机信息（P2.8「本机信息 API」，矩阵 §7.5）：喂设备页左栏「本机卡」。
        // 设备码/临时密码由本机态派生（`device_code_from` 稳定摘要 + `gen_temp_password`）；
        // mDNS 广播态 = 监听器是否起来了；`enc` 未接数据面探测，保守空值。
        .route(
            "/api/local-info",
            get(|State(ctx): State<Ctx>| async move {
                let cfg = ctx.store.load_or_init().unwrap_or_default();
                let mdns_advertising = ctx.listener.lock().unwrap().is_some();
                // 本机在线态由本机配置决定；不能按远端设备表/发现结果推断本机是否在线。
                let online = cfg.mdns_enabled;
                let device_code = device_code_from(&cfg.device_name);
                let temp_password = gen_temp_password();
                let info = local_info_of(&cfg, &device_code, &temp_password, vec![], mdns_advertising, online);
                Json(info).into_response()
            }),
        )
        // 日志查询（P2.8「日志查询 API」，矩阵 §7.5）：环形缓冲 + 级别/关键字过滤。
        .route(
            "/api/logs",
            get(|State(ctx): State<Ctx>, Query(q): Query<LogQuery>| async move {
                let level = q.level.unwrap_or_else(|| "ALL".into());
                let kw = q.q.unwrap_or_default();
                let lines = ctx.ring.lock().unwrap().export_lines(&level, &kw);
                Json(lines).into_response()
            }),
        )
        // 日志清空（P2.8「日志导出/清空 API」，矩阵 §7.5）。
        .route(
            "/api/logs/clear",
            post(|State(ctx): State<Ctx>| async move {
                ctx.ring.lock().unwrap().clear();
                StatusCode::NO_CONTENT.into_response()
            }),
        )
        // ---------- 会话命令面（P2 遗留 ①，矩阵 §7.5「会话建立/鉴权/断开 + 画面/键鼠/剪贴板/截图/显示器」）----------
        // 会话建立（FR-01 一键连接）：设备ID 经共享 DeviceTable 解析 ip:port，QUIC 拨号 + 握手。
        .route(
            "/api/session/connect",
            post(|State(ctx): State<Ctx>, Json(req): Json<SessionConnectReq>| async move {
                // 先取设备（std Mutex，非跨 await 持有），未命中 → 无路径。
                let dev: Option<OnlineDevice> = {
                    let mut devices = ctx.devices.lock().unwrap();
                    devices.find_online(&req.device_id, now_secs())
                };
                let sni = req.sni.unwrap_or_else(|| "localhost".into());
                let mut mgr = ctx.session.lock().await;
                match &dev {
                    Some(d) => match mgr.connect(d, &req.password, &sni).await {
                        Ok(v) => Json(SessionResp::ok(v)).into_response(),
                        Err(e) => {
                        tracing::warn!(%req.device_id, error = %e, "会话建立失败");
                        Json(SessionResp::fail(Some(mgr.status_view()), e.to_string())).into_response()
                        }
                    },
                    None => {
                        tracing::warn!(%req.device_id, "目标设备不在线（未发现 / 已过期）");
                        Json(SessionResp::fail(
                            Some(mgr.status_view()),
                            "device not online",
                        ))
                        .into_response()
                    }
                }
            }),
        )
        // 发送控制帧（FR-06 键鼠 / FR-07 剪贴板 / FR-04.5 编码 / FR-03.3 显示器）：意图展开成 1..N 帧逐帧发出。
        .route(
            "/api/session/control",
            post(|State(ctx): State<Ctx>, Json(req): Json<SessionControlReq>| async move {
                let mut mgr = ctx.session.lock().await;
                match mgr.send_control(&req.action).await {
                    Ok(sent) => {
                        let status = mgr.status_view();
                        Json(SessionResp::ok_sent(status, sent)).into_response()
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "发送控制帧失败");
                        Json(SessionResp::fail(Some(mgr.status_view()), e.to_string())).into_response()
                    }
                }
            }),
        )
        // 会话状态（FR-09.3）：出当前状态机 + 折叠快照（未连 = idle）。
        .route(
            "/api/session/status",
            get(|State(ctx): State<Ctx>| async move {
                let mgr = ctx.session.lock().await;
                Json(SessionResp::ok(mgr.status_view())).into_response()
            }),
        )
        // 会话关闭（FR-02.4）：优雅断开 + 状态机落 Closed。
        .route(
            "/api/session/close",
            post(|State(ctx): State<Ctx>| async move {
                let mut mgr = ctx.session.lock().await;
                mgr.close();
                Json(SessionResp::ok(mgr.status_view())).into_response()
            }),
        )
        // 视频画面推流（P2②，矩阵 §7.5「画面」）：把 `SessionManager` 视频泵
        // （`connect` 时起的后台重组任务，`recv_video_datagram` → `JitterBuffer` → NALU）
        // 出的 [`VideoFrameMsg`] 流式推给前端（SSE）。前端 `EventSource` 逐帧 `atob` 还原
        // NALU 喂 WebCodecs `VideoDecoder`。无活跃会话 → 404（前端据此保持占位/演示画面）。
        .route(
            "/api/session/video",
            get(|State(ctx): State<Ctx>| async move {
                let mut mgr = ctx.session.lock().await;
                let rx = mgr.take_video_rx();
                drop(mgr);
                // `take_video_rx` 移走泵入端（mpsc `Receiver`）；无会话为 `None` → 404。
                let Some(rx) = rx else {
                    tracing::warn!("视频推流：无活跃会话（先 connect 再拉画面）");
                    return (StatusCode::NOT_FOUND, "no active session").into_response();
                };
                // `mpsc::Receiver` → SSE 事件流：逐帧把 `VideoFrameMsg` 序列化成 JSON 走
                // `data:` 字段（NALU 为 base64 字符串），前端 `EventSource` `JSON.parse` 还原。
                // 泵结束 / 会话关闭时 `recv` 返回 `None` → 流 EOS。`Infallible` 错误型对齐
                // `axum::Sse`（本流只产 `Ok`）。
                let stream = stream::unfold(rx, |mut rx| async move {
                    let msg: VideoFrameMsg = rx.recv().await?;
                    let json = serde_json::to_string(&msg).ok()?;
                    Some((Ok(Event::default().data(json)) as Result<Event, Infallible>, rx))
                });
                Sse::new(stream).into_response()
            }),
        )
        // 静态托管 + SPA fallback（未匹配路径 → index.html）。
        .fallback(|State(ctx): State<Ctx>, req: Request| static_handler(ctx, req))
        .with_state(ctx);

    let listener = tokio::net::TcpListener::bind(args.bind).await.expect("bind 监听地址");
    tracing::info!(%args.bind, "ui-server 监听中（浏览器打开 http://{}/）", args.bind);
    axum::serve(listener, app).await.expect("serve");
}

/// 静态文件 handler（SPA 兜底）：按 URI 路径读 `static_dir` 下文件；缺文件 → `index.html`；
/// 仍无（前端未 build）→ 友好提示页。用 `std::fs` 直接读，避免引入 tower-http（保持 vendor 不变）。
async fn static_handler(ctx: Ctx, req: Request) -> Response {
    let (parts, _body) = req.into_parts();
    let path = parts.uri.path().trim_start_matches('/');

    if parts.method != Method::GET && parts.method != Method::HEAD {
        return (StatusCode::METHOD_NOT_ALLOWED, "GET only").into_response();
    }

    // 候选：先按原路径（资源文件），再 SPA 兜底 index.html。
    let candidates: Vec<PathBuf> = vec![
        ctx.static_dir.join(path),
        ctx.static_dir.join("index.html"),
    ];

    for cand in &candidates {
        if cand.is_file() {
            match std::fs::read(cand) {
                Ok(bytes) => {
                    return (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, mime_guess_by_ext(cand))],
                        bytes,
                    )
                        .into_response();
                }
                Err(e) => {
                    tracing::warn!(?e, ?cand, "读取静态文件失败");
                }
            }
        }
    }

    // 既无原路径也无 index.html（前端未 build）。给提示页而非裸 404。
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>LanDesk UI</title>\
        <body style='font:14px system-ui;padding:24px'>\
        <h2>LanDesk UI 服务已启动，但前端构建产物缺失。</h2>\
        <p>请先在仓库根执行 <code>cd ui/frontend &amp;&amp; npm install &amp;&amp; npm run build</code>，\
        再重启本服务。</p>\
        <p>静态目录：<code>{}</code></p>",
        ctx.static_dir.display()
    );
    (StatusCode::NOT_FOUND, body).into_response()
}

/// 极简 MIME 猜测（够前端用：html/js/css/json/svg/png/ico/woff）。
fn mime_guess_by_ext(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "map" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
