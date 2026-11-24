//! 控制端 CLI。
//!
//! 两个子命令供 headless 验证/自动化门禁使用（GUI 主能力不变）：
//! - `device-list`：mDNS 发现，输出 JSON 设备表（验证 FR-01）。
//! - `probe`：连一台设备并采样视频数据报，用带内 flags 判定画面可用性
//!   （是否收到关键帧 / 是否采集降级：无录屏权限 / 锁屏黑帧）。**不解码 H.264**，
//!   只解数据报头，故零编解码依赖，可在 CI/无显示环境跑。
//!
//! 原形态A骨架参数 `--target/--signaling` 保留兼容（仅打印提示，会话编排仍以 GUI 为准）。

use std::error::Error;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use lp::datagram::{capture_degraded_from_flags, NaluKind, VideoDgram};
use mdns::OnlineDevice;
use quic_link::QuicClient;
use serde::Serialize;

/// LanDesk 控制端（headless 验证入口）。
#[derive(Parser, Debug)]
#[command(name = "controller", about = "LanDesk 控制端：headless 设备发现与画面探针")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    /// 目标设备ID（外网模式，旧骨架兼容参数）。
    #[arg(long)]
    target: Option<String>,
    /// 信令地址（wss://host:port，旧骨架兼容参数）。
    #[arg(long)]
    signaling: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// mDNS 发现局域网在线设备，输出 JSON 数组。
    DeviceList {
        /// 发现窗口秒数。
        #[arg(long, default_value_t = 3)]
        wait_secs: u64,
    },
    /// 连接目标并采样视频流，判定画面可用性（PASS/FAIL + JSON）。
    Probe {
        /// 目标设备ID或设备名（取自 device-list）。
        #[arg(long)]
        device: String,
        /// 访问密码。
        #[arg(long)]
        password: String,
        /// TLS SNI（insecure 模式任意值，默认 localhost）。
        #[arg(long, default_value = "localhost")]
        sni: String,
        /// 采样时长秒数。
        #[arg(long, default_value_t = 5)]
        duration_secs: u64,
        /// 发现窗口秒数（连之前先 mDNS 发现）。
        #[arg(long, default_value_t = 3)]
        discover_secs: u64,
    },
}

#[derive(Debug, Default, Serialize)]
struct ProbeStats {
    /// 成功解码数据报头的包数（含分片）。
    packets: usize,
    /// IDR 关键帧包数（kind 0/1）。
    idr_packets: usize,
    /// SPS/PPS 包数（kind 3）。
    sps_pps_packets: usize,
    /// 数据报头解码失败数（非 LanDesk 包 / 版本不匹配）。
    decode_failures: usize,
    /// 收到首个视频包的耗时（毫秒）。
    first_packet_ms: Option<u128>,
    /// 采样窗口内出现的采集降级 reason（0=无录屏权限 / 1=锁屏黑帧）。
    degraded_reason: Option<u8>,
}

#[derive(Debug, Serialize)]
struct ProbeReport {
    verdict: &'static str,
    reason: &'static str,
    device: String,
    device_ip: String,
    device_port: u16,
    elapsed_ms: u128,
    stats: ProbeStats,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Some(Commands::DeviceList { wait_secs }) => device_list(wait_secs).await,
        Some(Commands::Probe {
            device,
            password,
            sni,
            duration_secs,
            discover_secs,
        }) => probe(&device, &password, &sni, duration_secs, discover_secs).await,
        None => {
            tracing::warn!(
                target = ?cli.target,
                signaling = ?cli.signaling,
                "未指定子命令：会话编排以 Tauri GUI 为准，headless 请用 device-list / probe"
            );
            Ok(())
        }
    }
}

/// 采样结果 → 门禁判据（纯函数，可单测）。
///
/// 优先级：采集降级 > 无帧 > 无关键帧 > 正常。
/// 只有 IDR（kind 0/1）才让前端解码器输出首帧；单独 SPS/PPS 重传不足以出画面，
/// 故 PASS 必须要求 `idr_packets > 0`，`sps_pps_packets` 仅作诊断统计。
fn verdict_for(stats: &ProbeStats) -> (&'static str, &'static str) {
    match stats.degraded_reason {
        Some(0) => ("FAIL", "capture_no_permission"),
        Some(1) => ("FAIL", "capture_blank_frame"),
        Some(_) => ("FAIL", "capture_degraded_unknown"),
        None if stats.packets == 0 => ("FAIL", "no_video_frames"),
        None if stats.idr_packets == 0 => ("FAIL", "no_keyframe"),
        None => ("PASS", "ok"),
    }
}

/// mDNS 发现：宿主侧验证「设备能否被扫描到」。
async fn device_list(wait_secs: u64) -> Result<(), Box<dyn Error>> {
    let mut listener = mdns::Listener::start()?;
    let devices = listener
        .discover_within(Duration::from_secs(wait_secs))
        .await?;
    println!("{}", serde_json::to_string_pretty(&devices)?);
    Ok(())
}

/// 连接 + 视频采样判定。
///
/// 判据（覆盖五问题中的「画面未出现 / 黑屏 / 降级」）：
/// - 连不上 → FAIL connect_failed / device_not_found；
/// - 无视频包 → FAIL no_video_frames（对端未起受控服务 / 防火墙拦入站也落这里）；
/// - 降级 flags 置位 → FAIL capture_no_permission / capture_blank_frame；
/// - 有包但无 IDR/SPS-PPS → FAIL no_keyframe（解码器无法启动，前端黑屏）；
/// - 否则 PASS。
async fn probe(
    device: &str,
    password: &str,
    sni: &str,
    duration_secs: u64,
    discover_secs: u64,
) -> Result<(), Box<dyn Error>> {
    let listener = mdns::Listener::start()?;
    let client = QuicClient::insecure_default("0.0.0.0:0".parse()?)?;
    let mut controller = landesk_core::Controller::new(Some(listener), client);

    let devices: Vec<OnlineDevice> = controller.discover(Duration::from_secs(discover_secs)).await?;
    let target = devices
        .iter()
        .find(|d| d.ad.id == device || d.ad.name == device || d.ad.instance_name() == device)
        .cloned();

    let Some(target) = target else {
        let report = ProbeReport {
            verdict: "FAIL",
            reason: "device_not_found",
            device: device.to_string(),
            device_ip: String::new(),
            device_port: 0,
            elapsed_ms: 0,
            stats: ProbeStats::default(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        std::process::exit(1);
    };

    let started = Instant::now();
    let session = match controller.connect(&target, password, sni).await {
        Ok(s) => s,
        Err(e) => {
            let report = ProbeReport {
                verdict: "FAIL",
                reason: "connect_failed",
                device: device.to_string(),
                device_ip: target.ip.clone(),
                device_port: target.port,
                elapsed_ms: started.elapsed().as_millis(),
                stats: ProbeStats { decode_failures: 1, ..Default::default() },
            };
            eprintln!("连接失败详情: {e}");
            println!("{}", serde_json::to_string_pretty(&report)?);
            std::process::exit(1);
        }
    };

    let rx = session.video_rx();
    let deadline = Instant::now() + Duration::from_secs(duration_secs);
    let mut stats = ProbeStats::default();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(buf)) => match VideoDgram::decode_plain(&buf) {
                Ok(dg) => {
                    if stats.packets == 0 {
                        stats.first_packet_ms = Some(started.elapsed().as_millis());
                    }
                    stats.packets += 1;
                    match NaluKind::from_u8(dg.kind) {
                        Some(NaluKind::IdrWithSpsPps) | Some(NaluKind::Idr) => stats.idr_packets += 1,
                        Some(NaluKind::SpsPps) => stats.sps_pps_packets += 1,
                        _ => {}
                    }
                    if let Some(r) = capture_degraded_from_flags(dg.flags) {
                        stats.degraded_reason = Some(r);
                    }
                }
                Err(_) => stats.decode_failures += 1,
            },
            Ok(Err(e)) => {
                eprintln!("视频接收中断: {e}");
                break;
            }
            Err(_) => break, // 采样窗口到
        }
    }
    session.close(0, b"probe done");

    let (verdict, reason) = verdict_for(&stats);

    let report = ProbeReport {
        verdict,
        reason,
        device: device.to_string(),
        device_ip: target.ip.clone(),
        device_port: target.port,
        elapsed_ms: started.elapsed().as_millis(),
        stats,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);

    if verdict == "FAIL" {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(packets: usize, idr: usize, degraded: Option<u8>) -> ProbeStats {
        ProbeStats {
            packets,
            idr_packets: idr,
            degraded_reason: degraded,
            ..Default::default()
        }
    }

    #[test]
    fn no_permission_degraded_fails() {
        assert_eq!(verdict_for(&stats(30, 2, Some(0))), ("FAIL", "capture_no_permission"));
    }

    #[test]
    fn blank_frame_degraded_fails() {
        assert_eq!(verdict_for(&stats(30, 2, Some(1))), ("FAIL", "capture_blank_frame"));
    }

    #[test]
    fn unknown_degraded_fails() {
        assert_eq!(verdict_for(&stats(30, 2, Some(9))), ("FAIL", "capture_degraded_unknown"));
    }

    #[test]
    fn zero_packets_fail() {
        assert_eq!(verdict_for(&stats(0, 0, None)), ("FAIL", "no_video_frames"));
    }

    #[test]
    fn sps_pps_alone_is_not_enough() {
        let mut s = stats(20, 0, None);
        s.sps_pps_packets = 20; // 只有参数集重传，解码器仍无 IDR 可出首帧
        assert_eq!(verdict_for(&s), ("FAIL", "no_keyframe"));
    }

    #[test]
    fn real_keyframe_passes() {
        assert_eq!(verdict_for(&stats(60, 3, None)), ("PASS", "ok"));
    }

    #[test]
    fn degraded_takes_priority_over_missing_frames() {
        assert_eq!(verdict_for(&stats(0, 0, Some(1))), ("FAIL", "capture_blank_frame"));
    }
}
