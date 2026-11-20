//! M6「信令注册联通」环回集成测试（对齐 `docs/architecture.md` §3 / `protocol.md` §3.1、FR-10）。
//!
//! 起真实 axum 信令服务（`/ws`）→ 两个 `signaling::Client` 各连一条 WS →
//! 双方 `reg` 注册 → A 发 `req{ id: B, from: A }` → 撮合后 A、B **各**收到一条
//! `peer`（含对方端点槽）。证明「轻 WS 信令把两端撮合联通」的最小闭环。

use std::net::SocketAddr;
use std::sync::Arc;

use signaling::{Client, SignalingMsg};
use signaling_server::router;
use tokio::sync::Mutex;
use signaling_server::Registry;

async fn start_server() -> (SocketAddr, Arc<Mutex<Registry>>) {
    let registry = Arc::new(Mutex::new(Registry::default()));
    let app = router(registry.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind 127.0.0.1:0");
    let addr = listener.local_addr().expect("listen addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, registry)
}

fn reg(id: &str, name: &str, relay: bool) -> SignalingMsg {
    SignalingMsg::Reg {
        id: id.into(),
        name: name.into(),
        caps: vec!["openh264".into(), "qsv".into()],
        res: "1920x1080".into(),
        relay,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_loopback_reg_then_req_yields_peer_both_sides() {
    let (addr, _registry) = start_server().await;
    let url = format!("ws://{addr}/ws");

    let mut a = Client::connect(&url).await.expect("A connect");
    let mut b = Client::connect(&url).await.expect("B connect");

    // 双方注册。
    a.send(&signaling::ClientOut::Reg(reg("dev-A", "a-board", true)))
        .await
        .expect("A reg");
    b.send(&signaling::ClientOut::Reg(reg("dev-B", "b-board", false)))
        .await
        .expect("B reg");

    // 给两条 reg 落地留一拍（内存在线表异步登记）。
    tokio::time::sleep(tokio::time::Duration::from_millis(80)).await;

    // A 请求连 B。
    a.send(&signaling::ClientOut::Req(SignalingMsg::Req {
        id: "dev-B".into(),
        from: "dev-A".into(),
    }))
    .await
    .expect("A req");

    // A、B 各收到一条 peer（撮合双向下发）。
    let (got_a, got_b) = tokio::join!(a.recv(), b.recv());
    match got_a {
        Ok(signaling::ClientIn::Peer(SignalingMsg::Peer { ref peer, .. })) => {
            assert_eq!(peer, "dev-B", "A 的 peer 指向 B");
        }
        other => panic!("A 期望 Peer，实际 {other:?}"),
    }
    match got_b {
        Ok(signaling::ClientIn::Peer(SignalingMsg::Peer { ref peer, .. })) => {
            assert_eq!(peer, "dev-A", "B 的 peer 指向 A");
        }
        other => panic!("B 期望 Peer，实际 {other:?}"),
    }

    let _ = (a.close().await, b.close().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_loopback_offline_target_yields_no_peer() {
    let (addr, _registry) = start_server().await;
    let url = format!("ws://{addr}/ws");

    let mut a = Client::connect(&url).await.expect("A connect");
    a.send(&signaling::ClientOut::Reg(reg("dev-A", "a-board", true)))
        .await
        .expect("A reg");
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // 请求不存在的目标 → 无撮合 → 服务端不下发任何 peer。
    a.send(&signaling::ClientOut::Req(SignalingMsg::Req {
        id: "dev-Ghost".into(),
        from: "dev-A".into(),
    }))
    .await
    .expect("A req ghost");

    // 200ms 内应收不到任何下行（无 peer / 无 ice）。
    let res = tokio::time::timeout(tokio::time::Duration::from_millis(200), a.recv()).await;
    match res {
        Err(_) => {
            // 超时 = 无任何下行，符合预期。
        }
        Ok(Ok(signaling::ClientIn::Peer(m))) => {
            panic!("离线目标不应撮合出 peer，实际 {m:?}");
        }
        Ok(other) => panic!("离线目标不应产生下行，实际 {other:?}"),
    }
    let _ = a.close().await;
}

