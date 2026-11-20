//! P2② 视频数据面发流泵回环测试（受控端发流 → 真 QUIC → 控制端重组还原像素）。
//!
//! 走 127.0.0.1 端到端：`Controlled` 起 QUIC 服务端 + `wait` 握手，`Controller`
//! （insecure client 信任自签证书）`connect` 建会话；受控端 [`controlled::VideoPump`]
//! 「合成采集 → 占位编码 → NALU 分片 → `send_video`」发帧，控制端 `recv_video` +
//! [`lp::jitter::JitterBuffer`] 重组还原 NALU，校验与泵产出的帧字节一致——证明
//! 发流泵 → 分片 → QUIC Datagram → 重组 全链路在真 QUIC 上跑通（对齐 `dataface.rs` 断言模式）。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use controlled::{PumpConfig, VideoPump};
use landesk_core::{Controlled, Controller};
use lp::datagram::{NaluKind, VideoDgram};
use lp::jitter::JitterBuffer;
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

/// 发 N 帧并逐帧 `recv_video` 收集所有 Datagram 分片，喂 `JitterBuffer` 重组，
/// 校验每条还原 NALU 与泵产出一致、帧序号 / 类型 / IDR 首帧判定正确。
#[tokio::test(flavor = "multi_thread")]
async fn video_pump_loopback_reassembles_frames() {
    // ---- 受控端：QUIC 服务端（自签证书）----
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let controlled =
        Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
    let port = controlled.server().local_addr().expect("listen addr").port();

    // ---- 控制端：insecure client 信任任意证书（环回自签）----
    let client =
        QuicClient::insecure_default("127.0.0.1:0".parse().expect("127.0.0.1:0")).expect("client");
    let controller = Controller::new(None, client);
    let dev = device(port);

    // ---- 并发握手：client connect + server wait ----
    let (c_session, s_session) =
        tokio::try_join!(
            controller.connect(&dev, PASSWORD, "localhost"),
            controlled.wait(PASSWORD),
        )
        .expect("connect + wait");

    // ---- 受控端起发流泵：合成 32×24 画面 + 占位编码，fps=0 不节流 ----
    let mut pump =
        VideoPump::new(32, 24, PumpConfig { kind: 9, fps: 0 }).expect("pump open");
    let frames: Vec<_> = (0..3)
        .map(|_| pump.next_frame().expect("next frame"))
        .collect();
    // 首帧 IDR+SPS/PPS，其后 P 帧。
    assert_eq!(frames[0].kind, NaluKind::IdrWithSpsPps as u8, "首帧强制 IDR");
    assert_eq!(frames[1].kind, NaluKind::PFrame as u8, "次帧起 P 帧");

    // 逐帧分片发送（`send` 内 `seq` 递增 + `JitterBuffer` 对侧重组）。
    for vf in &frames {
        let frags = pump.send(&s_session, vf).expect("send frame");
        assert!(frags >= 1, "每帧至少 1 片");
    }

    // ---- 控制端：收全部视频 Datagram，逐片喂 JitterBuffer 重组（3 帧，末片成帧）----
    let mut jb = JitterBuffer::new();
    let mut got_seqs: Vec<u16> = Vec::new();
    let mut got_kinds: Vec<u8> = Vec::new();
    let mut got_nalus: Vec<Vec<u8>> = Vec::new();
    while got_nalus.len() < frames.len() {
        let raw = c_session.recv_video().await.expect("recv video dgram");
        let d = VideoDgram::decode_plain(&raw).expect("decode video dgram");
        if let Some(frame) = jb.push(&d) {
            got_seqs.push(frame.seq);
            got_kinds.push(frame.kind);
            got_nalus.push(frame.nalu);
        }
    }

    // 逐帧校验：帧序号（pump.seq 从 0 起、send 时 ++ → 1..=N）、类型、NALU 字节往返一致。
    assert_eq!(got_nalus.len(), frames.len(), "3 帧全还原");
    for i in 0..frames.len() {
        assert_eq!(got_seqs[i], i as u16 + 1, "帧序号 1..=N（第 {i} 帧）");
        assert_eq!(got_kinds[i], frames[i].kind, "帧类型逐帧一致（第 {i} 帧）");
        assert_eq!(got_nalus[i], frames[i].nalu, "NALU 字节往返一致（第 {i} 帧）");
    }
}
