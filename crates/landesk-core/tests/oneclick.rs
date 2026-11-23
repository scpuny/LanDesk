//! M5「双机内网一键连接 + ECDH 会话升级」环回集成测试（对齐 `docs/protocol.md` §0.1/§0.2、
//! `architecture.md` §3）。
//!
//! 走 127.0.0.1 的端到端闭环（**不**起真实 mDNS 守护——多播需网卡，CI 容器常不可用）：
//! 1. [`Controlled`] 在 127.0.0.1:0 起 QUIC 服务端（自签证书），`wait(password)` 等握手；
//! 2. [`Controller`] 信任同一自签证书（环回同机可拿到），`connect(dev, password, "localhost")`；
//! 3. bootstrap GCM **认证隧道**：C 发 `AuthRequest`（prefix_cs）→ S 回 `AuthResult`（prefix_sc）；
//! 4. **ECDH 会话升级**：隧道内交换 X25519 公钥 + nonce + PBKDF2 proof，两侧
//!    [`session_keys`]（ECDH+HKDF）独立派生前向保密会话密钥，会话控制帧 / 视频 Datagram
//!    全部切到它（不再用 bootstrap 密钥）；
//! 5. 会话控制帧双向（Heartbeat / Ping）；
//! 6. 视频 Datagram 双向（H.264 NALU 明文包）；
//! 7. 优雅关闭。
//!
//! 并发模型与 M3 环路测试一致：双端点各带 UDP drive 任务 + QUIC 流按需通知 →
//! `multi_thread` runtime + `try_join!` 让控制端与受控端并发，避免顺序死锁。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use landesk_core::{derive_bootstrap, CoreError, Controlled, Controller, SessionState};
use lp::datagram::VideoDgram;
use lp::msg::{ControlMsg, Ping};
use mdns::{DeviceAd, OnlineDevice};
use quic_link::QuicClient;

const DEV_ID: &str = "8f3a1b2c9d0e";
const PASSWORD: &str = "s3cret";

fn device(port: u16) -> OnlineDevice {
    OnlineDevice {
        ad: DeviceAd {
            id: DEV_ID.into(),
            name: "devboard".into(),
            enc: "openh264".into(),
            res: "1920x1080".into(),
            ver: 1,
            relay: false,
        },
        ip: "127.0.0.1".into(),
        port,
        last_seen: 0,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn oneclick_loopback_roundtrip() {
    // ---- 受控端：起 QUIC 服务端（自签证书）----
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let controlled = Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
    let port = controlled.server().local_addr().expect("listen addr").port();

    // ---- 控制端：信任同一自签证书（环回同机可直接拿到）----
    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        controlled.server().tls(),
    )
    .expect("client init");
    let controller = Controller::new(None, client);
    let dev = device(port);

    // ---- 并发握手：client connect + server wait（QUIC 双端点须同时被驱动）----
    let (mut c_session, mut s_session) = tokio::try_join!(
        controller.connect(&dev, PASSWORD, "localhost"),
        controlled.wait(PASSWORD),
    )
    .expect("connect + wait");

    // 会话进入 Running，远端地址互通。
    assert_eq!(c_session.state(), SessionState::Running);
    assert_eq!(s_session.state(), SessionState::Running);
    assert_eq!(c_session.remote_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(s_session.remote_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));

    // ECDH 会话升级：会话密钥必须是前向保密的 ECDH 派生值，**不同于**
    // bootstrap 认证隧道密钥（隧道只用于握手帧 + 密码门禁）。双端一致。
    let bootstrap = derive_bootstrap(DEV_ID, PASSWORD);
    assert_ne!(c_session.enc_key, bootstrap.enc_key, "会话密钥应升级离 bootstrap 密钥");
    assert_eq!(c_session.enc_key, s_session.enc_key, "两侧 ECDH 派生会话密钥一致");

    // ---- 会话控制帧双向：client 发 Heartbeat，server 收后回 Ping ----
    let (client_got, server_got) = tokio::try_join!(
        async {
            c_session.send_control(&ControlMsg::Heartbeat(0x5A)).await?;
            // client 侧返回：收到 server 回的 Ping
            c_session.recv_control().await
        },
        async {
            // server 侧返回：收到 client 发的 Heartbeat
            let hb = s_session.recv_control().await?;
            s_session.send_control(&ControlMsg::Ping(Ping { ts: 9999 })).await?;
            Ok::<_, CoreError>(hb)
        },
    )
    .expect("control frames");

    assert!(matches!(client_got, ControlMsg::Ping(Ping { ts: 9999 })), "client 收到 Ping");
    assert!(matches!(server_got, ControlMsg::Heartbeat(0x5A)), "server 收到 Heartbeat");

    // ---- 视频 Datagram 双向 ----
    let vd1 = VideoDgram {
        seq: 10,
        kind: 0, // IdrWithSpsPps
        frag: 0x80,
        flags: 0x01, // FLAG_FIRST
        nalu: vec![0x01, 0x02, 0x03],
    };
    let vd2 = VideoDgram {
        seq: 11,
        kind: 2, // PFrame
        frag: 0x80,
        flags: 0,
        nalu: vec![0xAB, 0xCD, 0xEF],
    };
    let vd1p = vd1.encode_plain().expect("enc vd1");
    let vd2p = vd2.encode_plain().expect("enc vd2");

    let (vd2_at_client, vd1_at_server) = tokio::try_join!(
        async {
            c_session.send_video(&vd1p)?;
            c_session.recv_video().await
        },
        async {
            let got = s_session.recv_video().await?;
            s_session.send_video(&vd2p)?;
            Ok::<_, CoreError>(got)
        },
    )
    .expect("video dgrams");

    let d1 = VideoDgram::decode_plain(&vd1_at_server).expect("dec vd1");
    assert_eq!(d1.seq, 10, "server 收到的 I 帧序号");
    assert_eq!(d1.nalu, vec![0x01, 0x02, 0x03], "NALU 内容往返");
    let d2 = VideoDgram::decode_plain(&vd2_at_client).expect("dec vd2");
    assert_eq!(d2.seq, 11, "client 收到的 P 帧序号");
    assert_eq!(d2.nalu, vec![0xAB, 0xCD, 0xEF]);

    // ---- 优雅关闭 ----
    c_session.close(0, b"done");
    s_session.close(0, b"done");
}

/// 密码错 → 两侧 bootstrap `enc_key` 不一致（认证隧道密钥由「设备ID + 密码」派生）
/// → 受控端解不开 `AuthRequest` 的 GCM tag（或 PBKDF2 proof 不匹配）→ `Auth` 错误。
#[tokio::test(flavor = "multi_thread")]
async fn oneclick_wrong_password_rejected() {
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let controlled = Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
    let port = controlled.server().local_addr().expect("listen addr").port();

    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        controlled.server().tls(),
    )
    .expect("client init");
    let controller = Controller::new(None, client);
    let dev = device(port);

    // client 用错密码，server 用正确密码 → 两侧 enc_key 不同 → 握手帧解不开。
    let err = tokio::try_join!(
        controller.connect(&dev, "wrong-pw", "localhost"),
        controlled.wait(PASSWORD),
    )
    .expect_err("wrong password must fail handshake");

    // 至少一端以鉴权/握手类错误落败（密码错 → GCM tag 失败或连接异常）。
    let msg = format!("{err}");
    assert!(
        msg.contains("auth") || msg.contains("handshake") || msg.contains("quic"),
        "错密码应落入 auth/handshake/quic 错误，实际: {msg}"
    );
}

/// 多候选门禁：客户端用候选表**第二位**的密码连（首位是访问密码），
/// 命中非首位候选也应放行——验证「访问密码 OR 临时密码任一通过」。
#[tokio::test(flavor = "multi_thread")]
async fn wait_any_hits_non_first_candidate() {
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let controlled = Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
    let port = controlled.server().local_addr().expect("listen addr").port();

    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        controlled.server().tls(),
    )
    .expect("client init");
    let controller = Controller::new(None, client);
    let dev = device(port);

    // 受控端候选表：[访问密码, 临时码]。客户端用临时码（第二位）连接。
    let candidates = vec!["access-pw".to_string(), "temp-123".to_string()];
    let (_, s_session) = tokio::try_join!(
        controller.connect(&dev, "temp-123", "localhost"),
        controlled.wait_any(&candidates),
    )
    .expect("connect + wait_any must hit the temp code candidate");

    assert_eq!(s_session.state(), SessionState::Running, "命中第二位候选应建会话");
    s_session.close(0, b"done");
}

/// 多候选门禁：客户端用的密码**不在**候选表内 → 所有候选 GCM 都不过 → 拒绝。
#[tokio::test(flavor = "multi_thread")]
async fn wait_any_rejects_when_no_candidate_matches() {
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let controlled = Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
    let port = controlled.server().local_addr().expect("listen addr").port();

    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        controlled.server().tls(),
    )
    .expect("client init");
    let controller = Controller::new(None, client);
    let dev = device(port);

    // 候选表里没有 "nope" → 应全候选失败。
    let candidates = vec!["access-pw".to_string(), "temp-123".to_string()];
    let err = tokio::try_join!(
        controller.connect(&dev, "nope", "localhost"),
        controlled.wait_any(&candidates),
    )
    .expect_err("no matching candidate must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("auth") || msg.contains("quic"),
        "无候选命中应落入 auth/quic 错误，实际: {msg}"
    );
}
