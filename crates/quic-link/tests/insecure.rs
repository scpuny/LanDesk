//! M4 跨机一键模型环回：trust-any 客户端（拿不到对端证书）连自签证书服务端。
//!
//! 证明「控制端无法预置受控端证书时仍能完成 QUIC 握手」——内网双机一键连接
//! 的 TLS 通道前提（端到端机密性仍由会话 GCM 承担，见 `docs/protocol.md` §0）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use lp::msg::ControlMsg;
use lp::wire::ControlFrame;
use quic_link::{LpGcm, QuicClient, QuicServer};

const ENC_KEY: [u8; 32] = [3u8; 32];
const PREFIX: [u8; 4] = [0u8; 4];

#[tokio::test(flavor = "multi_thread")]
async fn insecure_client_trust_any_handshake() {
    // 服务端：自签证书（客户端**不**把它放进信任根）。
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let server = QuicServer::bind(listen, &["10.0.0.7"]).expect("server bind");
    let server_addr = server.local_addr().expect("addr");

    // 客户端：trust-any（ring provider），对端是陌生自签证书 → 仍能握手。
    let provider = quinn::rustls::crypto::ring::default_provider();
    let client = QuicClient::insecure(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        &provider,
    )
    .expect("insecure client");

    let (c_conn, s_conn) =
        tokio::try_join!(client.connect(server_addr, "10.0.0.7", ENC_KEY), server.accept(ENC_KEY),)
            .expect("insecure connect + accept");

    // 控制流：client 发一帧、server 收（证明 trust-any 通道真通了，不只是 TCP）。
    let gcm_c = LpGcm::with_prefix(ENC_KEY, PREFIX);
    let gcm_s = LpGcm::with_prefix(ENC_KEY, PREFIX);
    let hb = ControlFrame {
        msg: ControlMsg::Heartbeat(0x77),
        seq: 1,
    };
    let hb_cipher = ControlFrame::encrypt(&gcm_c, &ENC_KEY, &hb).expect("enc");

    let _got = tokio::try_join!(
        async {
            let (mut send, _recv) = c_conn.open_control_stream().await?;
            c_conn.write_control_cipher(&mut send, &hb_cipher).await?;
            Ok::<_, quic_link::QuicLinkError>(())
        },
        async {
            let (_send, mut recv) = s_conn.accept_control_stream().await?;
            let cipher = s_conn.read_control_cipher(&mut recv).await?;
            let plain =
                ControlFrame::decrypt(&gcm_s, &ENC_KEY, 1, |_| true, &cipher).expect("dec");
            assert!(matches!(plain.msg, ControlMsg::Heartbeat(0x77)));
            Ok::<_, quic_link::QuicLinkError>(())
        },
    )
    .expect("control over insecure channel");

    c_conn.close(0, b"done");
    s_conn.close(0, b"done");
    client.close();
    server.close();
}
