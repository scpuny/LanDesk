//! M3 环路测试：本机 QUIC 双通道（可靠控制流 + 不可靠视频 Datagram）往返。
//!
//! 覆盖 `docs/protocol.md` §1/§2 的端到端通路：
//! 1. 服务端 `bind` 127.0.0.1:0（自签证书）；
//! 2. 客户端信任同一证书并 `connect`；
//! 3. 控制流（一条 QUIC 双向流）：client 加密发 `Heartbeat` → server 解密验值；
//!    server 加密回 `Ping` → client 解密验值；
//! 4. 视频 Datagram：双向收发 `VideoDgram` 明文包并解码校验 NALU；
//! 5. 优雅关闭。
//!
//! 并发模型说明：
//! - client / server 各持一个独立 `Endpoint`（内部各带 UDP drive 任务），
//!   用 `multi_thread` runtime 让握手包交替被驱动；单线程下双端点握手会饿死超时。
//! - QUIC 双向流是「按需通知」：发起端 `open_bi` 后**必须 write** 到 SendStream，
//!   对端 `accept_bi` 才返回。故控制流交互用 `try_join!` 让 client（open+写+读）
//!   与 server（accept+读+写）真正并发，避免顺序死等。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use lp::datagram::VideoDgram;
use lp::msg::{ControlMsg, Ping};
use lp::wire::ControlFrame;
use quic_link::{LpGcm, QuicClient, QuicLinkError, QuicServer};

/// 环路测试共享的固定会话密钥（真实握手时来自 ECDH+HKDF）。
const ENC_KEY: [u8; 32] = [9u8; 32];
/// 两端共享的 GCM nonce 前缀（真实握手时由一端生成并经控制流下发）。
const GCM_PREFIX: [u8; 4] = [0u8; 4];

#[tokio::test(flavor = "multi_thread")]
async fn dual_channel_loopback_roundtrip() {
    // ---- 服务端 ----
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let server = QuicServer::bind(listen, &["localhost"]).expect("server bind");
    let server_addr = server.local_addr().expect("server addr");

    // ---- 客户端 ----
    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        server.tls(),
    )
    .expect("client init");

    // 并发：client 拨号 + server 接受（QUIC 握手需双方端点同时被驱动）。
    let (client_conn, server_conn) =
        tokio::try_join!(client.connect(server_addr, "localhost", ENC_KEY), server.accept(ENC_KEY),)
            .expect("connect + accept");

    // 地址互通校验。
    assert_eq!(client_conn.remote_addr(), server_addr);
    assert_eq!(server_conn.remote_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    // 两端 GCM 前缀一致（否则解不开）。
    assert_eq!(client_conn.gcm().prefix(), GCM_PREFIX);
    assert_eq!(server_conn.gcm().prefix(), GCM_PREFIX);

    let gcm_c = LpGcm::with_prefix(ENC_KEY, GCM_PREFIX);
    let gcm_s = LpGcm::with_prefix(ENC_KEY, GCM_PREFIX);

    // ---- 控制流：一条 QUIC 双向流 ----
    // client open + 写 hb + 读 ping；server accept + 读 hb + 写 ping。
    // 两边用 try_join! 并发，避免「client 写完阻塞读 ping」与「server 读完才写 ping」的顺序死锁。
    let hb = ControlFrame {
        msg: ControlMsg::Heartbeat(0x42),
        seq: 1,
    };
    let ping = ControlFrame {
        msg: ControlMsg::Ping(Ping { ts: 1_234 }),
        seq: 1,
    };
    let hb_cipher = ControlFrame::encrypt(&gcm_c, &ENC_KEY, &hb).expect("enc hb");
    let ping_cipher = ControlFrame::encrypt(&gcm_s, &ENC_KEY, &ping).expect("enc ping");

    let (ping_buf, hb_buf) = tokio::try_join!(
        async {
            let (mut c_send, mut c_recv) = client_conn.open_control_stream().await?;
            client_conn
                .write_control_cipher(&mut c_send, &hb_cipher)
                .await?;
            // client 侧返回：从 c_recv 读到 server 回的 ping 密文
            client_conn.read_control_cipher(&mut c_recv).await
        },
        async {
            let (mut s_send, mut s_recv) = server_conn.accept_control_stream().await?;
            let hb_buf = server_conn.read_control_cipher(&mut s_recv).await?;
            server_conn
                .write_control_cipher(&mut s_send, &ping_cipher)
                .await?;
            // server 侧返回：从 s_recv 读到 client 发的 hb 密文
            Ok::<_, QuicLinkError>(hb_buf)
        },
    )
    .expect("control stream");

    // server 解 hb：值与 seq 应一致。
    let hb_plain =
        ControlFrame::decrypt(&gcm_s, &ENC_KEY, 1, |_| true, &hb_buf).expect("dec hb");
    assert!(matches!(hb_plain.msg, ControlMsg::Heartbeat(0x42)), "heartbeat value");
    assert_eq!(hb_plain.seq, 1);

    // client 解 ping：ts 应一致。
    let ping_plain =
        ControlFrame::decrypt(&gcm_c, &ENC_KEY, 1, |_| true, &ping_buf).expect("dec ping");
    assert!(
        matches!(ping_plain.msg, ControlMsg::Ping(Ping { ts: 1_234 })),
        "ping ts"
    );

    // ---- 视频 Datagram：双向 ----
    // client 发 vd1 + 收 vd2；server 收 vd1 + 发 vd2。try_join! 并发。
    let vd1 = VideoDgram {
        seq: 7,
        kind: 0, // NaluKind::IdrWithSpsPps
        frag: 0x80, // 末片
        flags: 0x01, // FLAG_FIRST
        nalu: (0u8..=255).collect(),
    };
    let vd2 = VideoDgram {
        seq: 8,
        kind: 2, // PFrame
        frag: 0x80,
        flags: 0,
        nalu: vec![0xAB, 0xCD, 0xEF],
    };
    let vd1_plain = vd1.encode_plain().expect("enc vd1");
    let vd2_plain = vd2.encode_plain().expect("enc vd2");

    let (vd2_at_client, vd1_at_server) = tokio::try_join!(
        async {
            client_conn.send_video_datagram(&vd1_plain).expect("send vd1");
            // client 侧返回：收到的 vd2 明文
            client_conn.recv_video_datagram().await
        },
        async {
            let got1 = server_conn.recv_video_datagram().await?;
            server_conn.send_video_datagram(&vd2_plain).expect("send vd2");
            // server 侧返回：收到的 vd1 明文
            Ok::<_, QuicLinkError>(got1)
        },
    )
    .expect("video dgram");

    // server 解 vd1。
    let d1 = VideoDgram::decode_plain(&vd1_at_server).expect("dec vd1");
    assert_eq!(d1.seq, 7);
    assert!(d1.is_last(), "末片标志");
    assert_eq!(d1.nalu, vd1.nalu, "NALU 内容往返");

    // client 解 vd2。
    let d2 = VideoDgram::decode_plain(&vd2_at_client).expect("dec vd2");
    assert_eq!(d2.seq, 8);
    assert_eq!(d2.kind, 2);
    assert_eq!(d2.nalu, vec![0xAB, 0xCD, 0xEF]);

    // ---- 优雅关闭 ----
    client_conn.close(0u32, b"done");
    server_conn.close(0u32, b"done");
    client.close();
    server.close();
}
