//! M7 中继环回测试（FR-11，ADR-A3 字节透传）：
//!
//! 本机起 [`RelayEndpoint`]（QUIC 入向中继），两端 A/B 各建 [`QuicClient`]
//! （信任中继自签证书）拨号 → 同 ticket 出示 → 中继 [`RelayMatch`] 撮合 →
//! [`drive_matched`] 并发盲搬控制流 + 视频 Datagram。
//!
//! 流方向（每条物理连接 4 条 uni）：
//! - A 端 `open_uni`：ticket 流 + A→B 控制流；`accept_uni`：收 relay 开的 B→A 流；
//! - B 端 `open_uni`：ticket 流 + B→A 控制流；`accept_uni`：收 relay 开的 A→B 流；
//! - 中继（`rc_a`/`rc_b` = A/B 两条物理连接的服务端句柄）：`accept_uni` 收 A/B 的
//!   控制流，`open_uni` 把字节写给对端；视频 Datagram 靠 `read_datagram → send_datagram`
//!   盲搬（datagram per-connection，不会自动到对端）。
//!
//! 并发模型同 `loopback.rs`：A/B 各持独立 client endpoint（各带 UDP drive 任务），
//! `multi_thread` 让握手/透传包交替驱动；**握手期间须并发驱动中继 `accept`**
//! （QUIC 服务端握手包由 endpoint 驱动，`try_join!` 里加 `relay.accept()` 才不饿死）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bytes::Bytes;
use quic_link::{
    QuicClient, RelayEndpoint, RelayMatch, AddOutcome, drive_matched, read_ticket, write_ticket,
};

/// 环回占位会话密钥（中继不解密，只喂 `QuicClient::connect` 的 GCM 初始化）。
const ENC_KEY: [u8; 32] = [0u8; 32];
/// 两端共用 ticket（撮合键）。
const TICKET: &str = "tk-landesk-m7";

fn lo0() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}

/// 两端经同一 ticket 透传：控制流 + 视频 Datagram 双向盲搬全链路。
#[tokio::test(flavor = "multi_thread")]
async fn relay_two_peers_transparent() {
    // ---- 中继端点 ----
    let relay = RelayEndpoint::bind(lo0(), &["localhost"]).expect("relay bind");
    let relay_addr = relay.local_addr().expect("relay addr");

    // ---- 两端客户端（信任中继自签证书）----
    let ca = QuicClient::new(lo0(), relay.tls()).expect("client A");
    let cb = QuicClient::new(lo0(), relay.tls()).expect("client B");

    // 并发：A/B 拨号 + 中继 accept（驱动 QUIC 握手，避免 30s 超时）。
    let (qa, qb, rc_a, rc_b) = tokio::try_join!(
        ca.connect(relay_addr, "localhost", ENC_KEY),
        cb.connect(relay_addr, "localhost", ENC_KEY),
        relay.accept(),
        relay.accept(),
    )
    .expect("connect + accept both to relay");
    let a = qa.raw().clone();
    let b = qb.raw().clone();

    // ---- ticket 出示（client 侧 open uni）+ 中继侧读回 ----
    write_ticket(&a, TICKET).await.expect("A ticket");
    write_ticket(&b, TICKET).await.expect("B ticket");
    let ta = read_ticket(&rc_a).await.expect("read A ticket");
    let tb = read_ticket(&rc_b).await.expect("read B ticket");
    assert_eq!(ta, TICKET, "A 出示的 ticket");
    assert_eq!(tb, TICKET, "B 出示的 ticket");

    // ---- 中继撮合（relay 侧句柄进表；Connection: Clone 给 drive 任务）----
    let mut m = RelayMatch::default();
    let o1 = m.add(TICKET, rc_a.clone()).expect("add A");
    assert!(matches!(o1, AddOutcome::Waiting), "第一条等对端");
    let o2 = m.add(TICKET, rc_b.clone()).expect("add B");
    assert!(matches!(o2, AddOutcome::Matched(_, _)), "第二条撮合成功");
    let (pa, pb) = match o2 {
        AddOutcome::Matched(x, y) => (x, y),
        _ => unreachable!(),
    };
    assert_eq!(m.pending(), 0, "撮合后无残留未撮合连接");

    // ---- 驱动透传（中继侧并发泵 + 视频转发）----
    let drv = tokio::spawn(async move { drive_matched(&pa, &pb).await });

    // ---- A→B：控制流 + 视频 Datagram ----
    {
        let mut send = a.open_uni().await.expect("A ctrl stream");
        send.write_all(b"hello-from-A")
            .await
            .expect("A ctrl write");
        send.finish().expect("A ctrl finish");
    }
    let ab_video = Bytes::copy_from_slice(&[0xAB, 0xCD, 0xEF, 0x01]);
    a.send_datagram(ab_video.clone()).expect("A video dgram");

    // B 端收：控制流（relay 开的 A→B 流）+ 视频 datagram。
    let got_ctrl: Vec<u8> = {
        let mut recv = b.accept_uni().await.expect("B accept ctrl");
        recv.read_to_end(1024).await.expect("B ctrl read")
    };
    assert_eq!(got_ctrl, b"hello-from-A", "控制流原字节透传（不解密）");
    let got_video = b.read_datagram().await.expect("B video dgram");
    assert_eq!(got_video.to_vec(), ab_video.to_vec(), "视频 datagram 盲搬");

    // ---- B→A：控制流 + 视频 Datagram ----
    {
        let mut send = b.open_uni().await.expect("B ctrl stream");
        send.write_all(b"hi-from-B")
            .await
            .expect("B ctrl write");
        send.finish().expect("B ctrl finish");
    }
    let ba_video = Bytes::copy_from_slice(&[0x11, 0x22, 0x33]);
    b.send_datagram(ba_video.clone()).expect("B video dgram");

    let got_ctrl2: Vec<u8> = {
        let mut recv = a.accept_uni().await.expect("A accept ctrl");
        recv.read_to_end(1024).await.expect("A ctrl read")
    };
    assert_eq!(got_ctrl2, b"hi-from-B", "反向控制流透传");
    let got_video2 = a.read_datagram().await.expect("A video dgram");
    assert_eq!(got_video2.to_vec(), ba_video.to_vec(), "反向视频 datagram 盲搬");

    // ---- 收尾：关两端 → 透传任务退出 ----
    a.close(0u32.into(), b"done");
    b.close(0u32.into(), b"done");
    let _ = drv.await.expect("drive task");

    qa.close(0u32, b"done");
    qb.close(0u32, b"done");
    ca.close();
    cb.close();
    relay.close();
}

/// ticket 魔数不符 → 中继拒绝该连接（[`RelayError::TicketUnreadable`]）。
#[tokio::test(flavor = "multi_thread")]
async fn relay_rejects_bad_ticket() {
    let relay = RelayEndpoint::bind(lo0(), &["localhost"]).expect("relay bind");
    let relay_addr = relay.local_addr().expect("relay addr");

    let ca = QuicClient::new(lo0(), relay.tls()).expect("client");
    // 拨号 + accept 并发驱动握手。
    let (qa, rc) =
        tokio::try_join!(ca.connect(relay_addr, "localhost", ENC_KEY), relay.accept(),)
            .expect("connect + accept");
    let a = qa.raw().clone();

    // 直接 open_uni 写乱码（魔数错），不走 write_ticket。
    let mut send = a.open_uni().await.expect("open uni");
    send.write_all(b"\x00\x00\x00\x00 not-a-ticket")
        .await
        .expect("write");
    let _ = send.finish();

    let err = read_ticket(&rc).await.expect_err("魔数错应失败");
    assert!(
        matches!(err, quic_link::RelayError::TicketUnreadable),
        "应报 TicketUnreadable（实际 {err:?}）"
    );

    a.close(0u32.into(), b"done");
    qa.close(0u32, b"done");
    ca.close();
    relay.close();
}
