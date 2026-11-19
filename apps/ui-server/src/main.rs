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
    extract::{Request, State},
    http::{header, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use landesk_ui::{
    Config, ConfigStore, ConnMode, HoleResult, OnlineDevice, PathDecision, ReconnectScheduler,
    device_rows, decide_path, view_of,
};
use mdns::Listener;

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

/// 共享状态：mDNS 监听器 + 在线设备表 + 配置存储 + 静态目录。
/// 手动 `Clone`（`Listener` 非 Clone，用 `Arc<Mutex<…>>` 共享）。
#[derive(Clone)]
struct Ctx {
    listener: Arc<Mutex<Option<Listener>>>,
    devices: Arc<Mutex<Vec<OnlineDevice>>>,
    store: ConfigStore,
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

/// `/api/choose-path` 出参。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ChoosePathResp {
    Use { conn_mode: &'static str },
    NoPath,
}

/// `/api/reconnect` 入参（camelCase 对齐前端）。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReconnectReq {
    used_attempts: u32,
}

/// 当前 unix 时间（秒）。
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 路径决策结果 → REST 出参。
fn decide_resp(decision: &PathDecision) -> ChoosePathResp {
    match decision {
        PathDecision::Use(m) => ChoosePathResp::Use {
            conn_mode: match m {
                ConnMode::Inner => "inner",
                ConnMode::P2p => "p2p",
                ConnMode::Relay => "relay",
            },
        },
        PathDecision::NoPath => ChoosePathResp::NoPath,
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt().init();

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

    let ctx = Ctx {
        listener: Arc::new(Mutex::new(listener)),
        devices: Arc::new(Mutex::new(Vec::new())),
        store: ConfigStore::new(),
        static_dir: static_dir.clone(),
    };

    // 后台任务：周期驱动 mDNS 事件 + 刷新在线表 + TTL 剪枝（FR-01.4）。
    {
        let ctx = ctx.clone();
        let tick = std::time::Duration::from_secs(args.discover.max(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tick).await;
                // 非阻塞取走已到达事件，刷新设备表。
                if let Some(l) = ctx.listener.lock().unwrap().as_mut() {
                    while l.poll_next().is_some() {}
                }
                let mut devs = ctx.devices.lock().unwrap();
                let removed = mdns::prune_stale(&mut devs, now_secs());
                if removed > 0 {
                    tracing::debug!(removed, "mDNS 剪枝过期设备");
                }
            }
        });
    }

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        // 设备列表（FR-01.2）：当前在线表 + TTL 剪枝 + 排序行。
        .route(
            "/api/devices",
            get(|State(ctx): State<Ctx>| async move {
                let now = now_secs();
                let mut devs = ctx.devices.lock().unwrap().clone();
                Json(device_rows(&mut devs, now))
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
        // 连接路径决策（FR-05/FR-11）。
        .route(
            "/api/choose-path",
            post(|Json(req): Json<ChoosePathReq>| async move {
                let d = decide_path(req.inner_found, &req.hole, req.relay_enabled);
                Json(decide_resp(&d)).into_response()
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
