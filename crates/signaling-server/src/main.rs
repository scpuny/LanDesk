//! 轻信令服务（FR-10，独立二进制）。
//!
//! M6 落地：axum `/ws` 端点 + 内存在线表（`HashMap<device_id, 项>`，无数据库，FR-10.2），
//! reg/req/peer/ice/relay 五类消息撮合；可选内嵌 STUN 反射器（§3.2，避免第三方 STUN）。
//! 数据面（QUIC 双通道）已在 M3 落地，信令只交换「对方端点 + ICE 候选」，零画面流量。

use std::net::SocketAddr;

use clap::Parser;

#[derive(Parser)]
#[command(name = "signaling-server", about = "LanDesk 轻信令服务（axum + WebSocket，内存在线表）")]
struct Args {
    /// HTTP/WS 监听地址。
    #[arg(long, default_value = "127.0.0.1:8443")]
    bind: SocketAddr,
    /// 内嵌 STUN 反射器 UDP 地址（`-stun`，§3.2，NFR-03 避免第三方 STUN）。可空。
    #[arg(long)]
    stun: Option<SocketAddr>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();
    let args = Args::parse();
    tracing::info!(bind = %args.bind, stun = ?args.stun, "signaling-server 启动");
    // 阻塞服务（/ws 撮合 + 可选 STUN 反射）；进程信号由外层托管。
    signaling_server::serve(args.bind, args.stun).await?;
    Ok(())
}
