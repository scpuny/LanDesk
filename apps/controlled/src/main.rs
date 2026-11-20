//! 受控端 CLI / 服务（形态A独立单文件二进制，ADR-A6 双角色同二进制）。
//!
//! P2② 起接**视频数据面发流**（矩阵 §7.5「画面」）：`Controlled` 起 QUIC 服务端
//! （可选 mDNS 广播）→ 密码门禁 + ECDH 会话升级 [`landesk_core::Controlled::wait`] →
//! 起 [`controlled::VideoPump`] 发流泵，按 fps 节拍「合成采集 → 占位编码 → NALU 分片 →
//! QUIC 视频 Datagram」把画面推到控制端（真平台采集/硬编按 feature 后续替换合成源与占位编码）。

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use controlled::{PumpConfig, VideoPump};
use landesk_core::config::Config;
use landesk_core::Controlled;
use tracing::warn;

/// LanDesk 受控端。
#[derive(Parser, Debug)]
#[command(name = "controlled", about = "LanDesk 受控端（被远程访问的主机）")]
struct Cli {
    /// 配置文件路径（`~/.config/landesk/config.toml`）。
    #[arg(long)]
    config: Option<PathBuf>,
    /// QUIC 监听地址（`ip:port`，默认 `0.0.0.0:52000`）。
    #[arg(long, default_value = "0.0.0.0:52000")]
    listen: SocketAddr,
    /// 访问密码（控制端须持同一密码经 PBKDF2 证明通过门禁）。
    #[arg(long, default_value = "landesk")]
    password: String,
    /// 设备ID（12 位 hex，握手 + 密钥派生用）。
    #[arg(long, default_value = "8f3a1b2c9d0e")]
    device: String,
    /// 画面分辨率（`WxH`，喂视频泵合成源）。
    #[arg(long, default_value = "1280x720")]
    res: String,
    /// 目标帧率。
    #[arg(long, default_value_t = 30)]
    fps: u32,
}

/// 解析 `WxH` 分辨率（缺省 `1280x720`）。
fn parse_res(s: &str) -> (u32, u32) {
    let (w, h) = match s.split_once('x') {
        Some((w, h)) => (w.parse().unwrap_or(1280), h.parse().unwrap_or(720)),
        None => (1280, 720),
    };
    (w.max(1), h.max(1))
}

/// 起一次视频发流泵循环：按 fps 节拍 `next()` 取帧 → `send(session, frame)` 分片发送，
/// 直到会话 `send_video` 失败（对端关闭 / 连接掉线）即停。
async fn run_pump(pump: &mut VideoPump, session: &landesk_core::OneClickSession) {
    let iv = pump.interval();
    loop {
        let vf = match pump.next_frame() {
            Ok(vf) => vf,
            Err(e) => {
                warn!(error = %e, "采集/编码失败，停发流泵");
                return;
            }
        };
        // 发流失败（连接已关）即停；分片数记日志供可观测。
        match pump.send(session, &vf) {
            Ok(n) => tracing::debug!(%n, "发出视频分片"),
            Err(e) => {
                warn!(error = %e, "send_video 失败，会话结束");
                return;
            }
        }
        if let Some(d) = iv {
            tokio::time::sleep(d).await;
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let _config = Config::default(); // 配置加载骨架（后续接 config 路径）
    let _ = cli.config;
    tracing_subscriber::fmt().init();
    tracing::info!(target: "controlled", ?cli, "受控端启动（P2② 接视频数据面发流）");

    let listen = cli.listen;
    let (w, h) = parse_res(&cli.res);

    // 起 QUIC 服务端（自签证书，`localhost` + 设备ID 供控制端 SNI）。
    let controlled = match Controlled::new(listen, &["localhost"], cli.device.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "QUIC 服务端绑定失败");
            return;
        }
    };
    let port = controlled
        .server()
        .local_addr()
        .map(|a| a.port())
        .unwrap_or(cli.listen.port());
    tracing::info!(listen = %listen, %port, "QUIC 服务端监听中");

    // 等一条连接 + 密码门禁 + ECDH 会话升级。
    let session = match controlled.wait(&cli.password).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "握手失败（密码错 / 连接异常）");
            return;
        }
    };
    tracing::info!(remote = %session.remote_addr(), "会话建立，起视频发流泵");

    // 起发流泵（合成源 `w×h` + 占位编码，首帧 IDR+SPS/PPS，其后 P 帧）。
    let mut pump = match VideoPump::new(w, h, PumpConfig { kind: 9, fps: cli.fps }) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "编码器 open 失败");
            return;
        }
    };
    run_pump(&mut pump, &session).await;

    session.close(0, b"controlled shutdown");
    tracing::info!("受控端会话结束");
}
