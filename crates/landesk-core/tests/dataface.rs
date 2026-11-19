//! C3 数据面回环测试：打通「采集 → 编码 → VideoDgram → 真 QUIC Datagram → 还原」最薄链路。
//!
//! 与 M3 `loopback.rs`（控制流 + 手工构造 VideoDgram）不同，本测试走真实数据面三件套：
//! - `capture::SynthCapture` 逐帧产**真·变化像素**（C1）；
//! - `avcodec_wrap::PlaceholderEncoder` 把帧包成带 H.264 起始码的裸 NALU（C2，占 openh264 槽）；
//! - 包成 `lp::datagram::VideoDgram` 后经**真 QUIC 双端点不可靠 Datagram** 发送，
//!   对端 `recv_video_datagram` + `VideoDgram::decode_plain` 还原 NALU，
//!   校验像素往返无损、烧录帧号自洽。
//!
//! 证明数据面通路（FR-03 采集 / FR-04 编码 → FR-05 QUIC）在真实 QUIC 上端到端跑通，非 mock。
//! 分辨率取 16×16：帧 1024B，NALU=4B 起始码+1024B=1028B < `lp::DGRAM_MAX`(1232)，保证可单 Datagram 发送。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use avcodec_wrap::{NALU_START_CODE, PlaceholderEncoder, VideoEncoder};
use capture::{Monitor, ScreenCapture, SynthCapture};
use lp::datagram::{NaluKind, VideoDgram, FLAG_FIRST};
use quic_link::{QuicClient, QuicLinkError, QuicServer};

/// 环路测试共享的固定会话密钥（真实握手时来自 ECDH+HKDF）。
const ENC_KEY: [u8; 32] = [7u8; 32];
/// 测试画面尺寸（16×16 BGRA = 1024 字节/帧）。
const W: u32 = 16;
const H: u32 = 16;
/// 采集帧数。
const FRAMES: usize = 2;

/// 采集 `FRAMES` 帧合成画面 → 占位编码 → 返回 `(seq, nalu, 原始帧字节)`。
///
/// 非 async、无网络，单独可离线验证「采集→编码」段（见 [`encode_frames_produces_varied_payloads`]）。
fn encode_frames() -> Vec<(u16, Vec<u8>, Vec<u8>)> {
    let cap = SynthCapture::default().with_monitor(Monitor {
        index: 0,
        width: W,
        height: H,
        name_hash: 0,
    });
    let mut enc = PlaceholderEncoder::new();
    enc.open(W, H, 30).expect("encoder open");
    (0..FRAMES)
        .map(|i| {
            let frame = cap.grab(0).expect("grab frame");
            let pixels = frame.data.clone();
            enc.send_frame(&pixels).expect("send frame");
            let nalu = enc.recv_nalu().expect("encoded nalu");
            assert_eq!(
                nalu.len(),
                NALU_START_CODE.len() + pixels.len(),
                "NALU = 起始码 ++ 帧字节"
            );
            (i as u16, nalu, pixels)
        })
        .collect()
}

/// 非网络冒烟：编码段本身（采集帧 → 占位 NALU）可离线验证。
#[test]
fn encode_frames_produces_varied_payloads() {
    let f = encode_frames();
    assert_eq!(f.len(), FRAMES);
    assert_ne!(f[0].2, f[1].2, "连续两帧画面必须不同（合成源在动）");
    for (i, (_, _, px)) in f.iter().enumerate() {
        // 左上角 4 字节 = 烧录的帧号（LE），须与序号自洽。
        let burnt = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        assert_eq!(burnt, i as u32, "帧 {i} 烧录帧号自洽");
    }
}

/// 端到端：经真 QUIC 双端点不可靠 Datagram 发送编码出的 NALU，对端还原并校验。
#[tokio::test(flavor = "multi_thread")]
async fn dataface_roundtrip_over_real_quic() {
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let server = QuicServer::bind(listen, &["localhost"]).expect("server bind");
    let server_addr = server.local_addr().expect("server addr");
    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        server.tls(),
    )
    .expect("client init");

    // 发送端（server，模拟 controlled 端）先把帧采集编码打包成 VideoDgram 明文。
    let frames = encode_frames();
    let plains: Vec<Vec<u8>> = frames
        .iter()
        .map(|(seq, nalu, _)| {
            let vd = VideoDgram {
                seq: *seq,
                kind: NaluKind::PFrame as u8,
                frag: 0x80, // 单片末片
                flags: if *seq == 0 { FLAG_FIRST } else { 0 },
                nalu: nalu.clone(),
            };
            vd.encode_plain().expect("encode_plain")
        })
        .collect();

    // 建链（QUIC 握手需双端点并发驱动）。
    let (client_conn, server_conn) =
        tokio::try_join!(client.connect(server_addr, "localhost", ENC_KEY), server.accept(ENC_KEY),)
            .expect("connect + accept");

    // 数据面回环：client 收 2 个 datagram，server 发 2 个（并发避免顺序死等）。
    let (rcvd, _): (Vec<VideoDgram>, ()) = tokio::try_join!(
        async {
            let mut r = Vec::with_capacity(FRAMES);
            for _ in 0..FRAMES {
                let raw = client_conn.recv_video_datagram().await?;
                r.push(VideoDgram::decode_plain(&raw).expect("decode video dgram"));
            }
            Ok::<_, QuicLinkError>(r)
        },
        async {
            for p in &plains {
                server_conn.send_video_datagram(p)?;
            }
            Ok::<_, QuicLinkError>(())
        },
    )
    .expect("video datagrams");

    // 按 seq 归位后逐帧校验：NALU 无损、像素还原、烧录帧号 == seq。
    let mut by_seq: Vec<Option<VideoDgram>> = vec![None; FRAMES];
    for d in rcvd {
        let s = d.seq as usize;
        by_seq[s] = Some(d);
    }
    for i in 0..FRAMES {
        let d = by_seq[i].take().expect("frame received");
        assert!(d.is_last(), "末片标志");
        let expected_nalu = frames[i].1.clone();
        assert_eq!(d.nalu, expected_nalu, "frame {i} NALU 往返无损");
        let pixels = &d.nalu[NALU_START_CODE.len()..];
        assert_eq!(pixels, frames[i].2, "frame {i} 帧像素无损还原");
        let burnt = u32::from_le_bytes([
            pixels[0],
            pixels[1],
            pixels[2],
            pixels[3],
        ]);
        assert_eq!(burnt, i as u32, "frame {i} 烧录帧号自洽");
        assert_eq!(d.seq, i as u16, "VideoDgram.seq 与帧序一致");
    }

    // 优雅关闭。
    client_conn.close(0u32, b"done");
    server_conn.close(0u32, b"done");
    client.close();
    server.close();
}
