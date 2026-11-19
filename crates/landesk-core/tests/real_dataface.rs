//! 真数据面单测（FR-03 真截屏 + FR-04 真 H.264 编码 + FR-05 真 QUIC 回环 + 像素校验）。
//!
//! 与 [`dataface`](dataface.rs)（合成源 + 占位编码，纯逻辑可离线跑）不同，本测试用**系统工具**
//! 接**真后端**，全程 1 台机：
//!
//! ```text
//! import(X11 截 root window 160×120)
//!   → ffmpeg(libx264 真 H.264 编码, 产物 < lp::DGRAM_MAX)
//!   → lp::datagram::VideoDgram → 真 QUIC 双端点 send/recv_video_datagram
//!   → 对端 ffmpeg(libx264 解码) → PSNR 像素校验
//! ```
//!
//! 实现约束：
//! - 用 `std::process::Command` 调外部 `import`/`ffmpeg`，**不引入新 Rust 依赖**，
//!   `--offline --locked` 门与 CI 三平台矩阵不受影响（vendor/lock 不动）。
//! - 缺 `import`/`ffmpeg` 或无 `DISPLAY`（CI 无 X、无 X11）时**自动 skip 不 fail**（eprintln 说明后 return）。
//! - 截屏尺寸取 160×120（偶数）：实测 libx264 编码产物 ~757B < `lp::DGRAM_MAX`(1232)，
//!   单 QUIC datagram 可发，无需分片。
//! - **键鼠真注入（`/dev/uinput`）需真机**（容器无该设备），本测试不含，留 OK3568/桌面真机。
//!
//! PSNR 阈值取 20dB：有损 H.264 正常 25~40dB，纯色/静态屏更高；20dB 是宽松下限，
//! 既过滤「完全没还原出来」，又对屏幕内容变化鲁棒。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Command;

use lp::datagram::{NaluKind, VideoDgram, FLAG_FIRST};
use quic_link::{QuicClient, QuicLinkError, QuicServer};

/// 环路测试共享的固定会话密钥（真实握手时来自 ECDH+HKDF，见 dataface.rs）。
const ENC_KEY: [u8; 32] = [3u8; 32];
/// 截屏尺寸（偶数；libx264 编码产物 ~757B < lp::DGRAM_MAX，单 datagram 可发）。
const SNAP_W: u32 = 160;
const SNAP_H: u32 = 120;
/// 有损 H.264 像素还原阈值（PSNR dB）。
const PSNR_THRESHOLD: f32 = 20.0;

/// 唯一工作目录（防同进程重跑串味）。
fn workdir() -> PathBuf {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let d = std::env::temp_dir().join(format!("landesk-rdf-{}-{}", std::process::id(), ns));
    let _ = std::fs::create_dir_all(&d);
    d
}

/// import 截 X11 root window 的 160×120 左上角到 PNG。无 DISPLAY 或 import 缺失返 false。
fn screenshot_to_png(dst: &str) -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skip real_dataface: no DISPLAY (CI 无 X11)");
        return false;
    }
    let crop = format!("{SNAP_W}x{SNAP_H}+0+0");
    match Command::new("import")
        .args(["-window", "root", "-crop", &crop, "-quiet", dst])
        .output()
    {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            eprintln!(
                "skip real_dataface: import 截屏失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
            false
        }
        Err(e) => {
            eprintln!("skip real_dataface: import 不可用 ({e})");
            false
        }
    }
}

/// ffmpeg 真 H.264 编码（libx264，yuv420p，zerolatency）。ffmpeg 缺失返 false。
fn encode_h264(png: &str, h264: &str) -> bool {
    match Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            png,
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-tune",
            "zerolatency",
            h264,
        ])
        .output()
    {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            eprintln!(
                "skip real_dataface: ffmpeg 编码失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
            false
        }
        Err(e) => {
            eprintln!("skip real_dataface: ffmpeg 不可用 ({e})");
            false
        }
    }
}

/// ffmpeg 真 H.264 解码回 PNG。
fn decode_h264(h264: &str, png: &str) -> bool {
    Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i", h264, "-frames:v", "1", png])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// ffmpeg 计算两 PNG 的 PSNR（dB）；解析不到返回 None。
fn psnr(a: &str, b: &str) -> Option<f32> {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-i",
            a,
            "-i",
            b,
            "-lavfi",
            "psnr",
            "-f",
            "null",
            "-",
        ])
        .output()
        .ok()?;
    // psnr filter 的 `[Parsed_psnr_0] ... average:NN.NN` 行落在 stdout；兜底也查 stderr。
    let hay = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in hay.lines() {
        if let Some(idx) = line.find("average:") {
            let rest = line[idx + "average:".len()..].trim();
            // `average:inf`（近乎无损）或 `average:NN.NN`。token 取字母数字/正负号。
            let token: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '+')
                .collect();
            if token.eq_ignore_ascii_case("inf") {
                return Some(f32::INFINITY);
            }
            if let Ok(v) = token.parse::<f32>() {
                return Some(v);
            }
        }
    }
    None
}

/// 非网络冒烟：截屏 → 真 H.264 编码 → 真解码 → PSNR 校验（缺工具/DISPLAY 则 skip）。
#[test]
fn real_screenshot_encode_decode_smoke() {
    let d = workdir();
    let png = d.join("snap.png");
    let h264 = d.join("snap.h264");
    let dec = d.join("snap_dec.png");
    let (png, h264, dec) = (png.to_str().unwrap(), h264.to_str().unwrap(), dec.to_str().unwrap());

    let ok = screenshot_to_png(png)
        && encode_h264(png, h264)
        && decode_h264(h264, dec);
    if !ok {
        eprintln!("real_dataface smoke skipped（缺 import/ffmpeg/DISPLAY）");
        return;
    }
    let p = psnr(png, dec).expect("PSNR 可解析（ffmpeg 存在时）");
    eprintln!("真数据面冒烟 PSNR={p}dB（源 {SNAP_W}x{SNAP_H} X11 截屏）");
    assert!(
        p >= PSNR_THRESHOLD,
        "真 H.264 编解码像素还原 PSNR={p} 低于阈值 {PSNR_THRESHOLD}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// 端到端：截屏 → 真 H.264 编码 → VideoDgram → 真 QUIC 双端点 → 对端真解码 → PSNR 校验。
///
/// 证明**真截屏 + 真编码 + 真 QUIC + 真解码**在 1 台机上全链路跑通（非合成、非占位）。
#[tokio::test(flavor = "multi_thread")]
async fn real_dataface_roundtrip_over_real_quic() {
    let d = workdir();
    let png = d.join("snap.png");
    let h264 = d.join("snap.h264");
    let recv_h264 = d.join("recv.h264");
    let enc_dec = d.join("recv_dec.png");
    let png = png.to_str().unwrap().to_string();
    let h264 = h264.to_str().unwrap().to_string();
    let recv_h264 = recv_h264.to_str().unwrap().to_string();
    let enc_dec = enc_dec.to_str().unwrap().to_string();

    // 发送端：截屏 + 真编码（缺工具/DISPLAY → skip）。
    let ready = screenshot_to_png(&png) && encode_h264(&png, &h264);
    if !ready {
        eprintln!("skip real_dataface QUIC（缺 import/ffmpeg/DISPLAY）");
        let _ = std::fs::remove_dir_all(&d);
        return;
    }
    let nalu = std::fs::read(&h264).expect("读编码产物");
    assert!(
        nalu.len() <= lp::DGRAM_MAX,
        "编码产物 {}B 超 lp::DGRAM_MAX({}B)，单 datagram 发不下",
        nalu.len(),
        lp::DGRAM_MAX
    );

    // 组 VideoDgram（PFrame，单片末片），明文须可序列化。
    let vd = VideoDgram {
        seq: 0,
        kind: NaluKind::PFrame as u8,
        frag: 0x80,
        flags: FLAG_FIRST,
        nalu: nalu.clone(),
    };
    let plain = vd.encode_plain().expect("encode_plain");

    // 真 QUIC 双端点（仿 dataface.rs / loopback.rs 写法）。
    let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let server = QuicServer::bind(listen, &["localhost"]).expect("server bind");
    let server_addr = server.local_addr().expect("server addr");
    let client = QuicClient::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        server.tls(),
    )
    .expect("client init");

    let (client_conn, server_conn) =
        tokio::try_join!(client.connect(server_addr, "localhost", ENC_KEY), server.accept(ENC_KEY),)
            .expect("connect + accept");

    // server 发、client 收（并发避免顺序死等）。
    let got: Vec<u8> = tokio::try_join!(
        async { client_conn.recv_video_datagram().await },
        async {
            server_conn.send_video_datagram(&plain)?;
            Ok::<_, QuicLinkError>(())
        },
    )
    .expect("video datagram")
    .0;

    // 对端还原：NALU 经 QUIC 无损，且可被真解码。
    let dv = VideoDgram::decode_plain(&got).expect("decode video dgram");
    assert_eq!(dv.nalu, nalu, "真编码字节经 QUIC Datagram 无损");

    let dec_ok = std::fs::write(&recv_h264, &dv.nalu)
        .is_ok()
        && decode_h264(&recv_h264, &enc_dec);
    assert!(dec_ok, "对端 ffmpeg 真解码收到的 H.264");
    let p = psnr(&png, &enc_dec).expect("PSNR 可解析");
    eprintln!(
        "真 QUIC 数据面回环 PSNR={p}dB（{SNAP_W}x{SNAP_H} 截屏 {}B NALU 经真 QUIC）",
        nalu.len()
    );
    assert!(
        p >= PSNR_THRESHOLD,
        "真数据面 QUIC 回环像素还原 PSNR={p} 低于阈值 {PSNR_THRESHOLD}"
    );

    // 优雅关闭 + 清理。
    client_conn.close(0u32, b"done");
    server_conn.close(0u32, b"done");
    client.close();
    server.close();
    let _ = std::fs::remove_dir_all(&d);
}
