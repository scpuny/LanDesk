//! QUIC 中继服务（FR-11，独立二进制，ADR-A3 仅字节透传、不解密不存会话状态）。
//!
//! M7 落地：`RelayEndpoint` 起 QUIC 入向端点 → 每条连接 [`read_ticket`] 出示 ticket →
//! [`RelayMatch`] 撮合同 ticket 的两端 → [`drive_matched`] 并发盲搬控制流 + 视频
//! Datagram（会话密钥 E2E，中继不可见画面，NFR-03）。
//!
//! 用法：`relay-server --bind 0.0.0.0:9000`。

use std::net::SocketAddr;

use quic_link::{drive_matched, read_ticket, RelayEndpoint, RelayMatch, AddOutcome};

fn parse_args() -> SocketAddr {
    let args: Vec<String> = std::env::args().collect();
    let mut bind: Option<SocketAddr> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--bind" && i + 1 < args.len() {
            bind = args[i + 1].parse().ok();
            i += 2;
        } else {
            i += 1;
        }
    }
    bind.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 9000)))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();
    let bind = parse_args();

    let relay = match RelayEndpoint::bind(bind, &["localhost"]) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(%e, "RelayEndpoint 绑定失败");
            std::process::exit(1);
        }
    };
    let listen = relay.local_addr().expect("relay 已绑定，local_addr 必成");
    tracing::info!(%listen, "QUIC 中继启动（FR-11，ADR-A3 字节透传）");

    // 撮合主循环：每条入向连接读 ticket → 进表；撮合成功即对该对起透传任务。
    let mut match_table: RelayMatch = RelayMatch::default();
    loop {
        let conn = match relay.accept().await {
            Ok(c) => c,
            Err(_) => break, // 端点关闭（SIGTERM / close）
        };

        // ticket 出示失败（魔数错 / 流提前断）→ 直接关这条连接。
        let ticket = match read_ticket(&conn).await {
            Ok(t) => t,
            Err(_) => {
                tracing::warn!("入向连接未出示有效 ticket，关闭");
                conn.close(1u32.into(), b"bad ticket");
                continue;
            }
        };

        match match_table.add(&ticket, conn) {
            Ok(AddOutcome::Waiting) => {
                tracing::debug!(%ticket, "中继：等待同 ticket 的对端");
            }
            Ok(AddOutcome::Matched(a, b)) => {
                tracing::info!(%ticket, "中继：撮合成功，开透传通道");
                // 每条撮合对独立驱动；连接关闭即任务退出。
                tokio::spawn(async move {
                    let (cab, cba, vab, vba) = drive_matched(&a, &b).await;
                    tracing::debug!(%ticket, cab, cba, vab, vba, "中继：透传通道收尾");
                    a.close(0u32.into(), b"relay done");
                    b.close(0u32.into(), b"relay done");
                });
            }
            Err(e) => {
                // 第三条：`add` 已消费并丢弃该 conn 句柄（未入表）；这里只记录。
                // quinn 侧连接由端点管理，本中继不保留它。
                tracing::warn!(%ticket, %e, "中继：ticket 槽满，拒绝并丢弃第三条连接");
            }
        }
    }
    tracing::info!("中继服务退出");
}
