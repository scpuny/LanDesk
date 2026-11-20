//! 受控端视频数据面发流（P2②，矩阵 §7.5「画面」，FR-03/FR-04 链路最薄可验证段）。
//!
//! 打通「采集 → 编码 → NALU 分片 → QUIC 视频 Datagram」发流泵：
//! - 采集用 [`capture::SynthCapture`]（逐帧产真·变化像素，无 OS 依赖，离线可验证；
//!   真实平台采集 Graphics Capture/PipeWire/CGDisplayStream 按 feature 后续接入）；
//! - 编码用 [`avcodec_wrap::PlaceholderEncoder`]（Openh264 软编兜底槽，纯 Rust，不链接 ffmpeg）；
//! - NALU 经 [`lp::datagram::VideoDgram::fragment_nalu`] 切成 ≤MTU 分片（含 H.264 起始码前缀），
//!   逐片 `OneClickSession::send_video`（明文 `encode_plain` 输出）。
//!
//! 接收端对侧走 `landesk-ui` 视频泵（`recv_video_datagram` → [`lp::jitter::JitterBuffer`] 重组），
//! 两端在真 QUIC 上回环校验像素（见 `landesk-core/tests/dataface.rs` 同款断言模式）。

use std::time::Duration;

use avcodec_wrap::{PlaceholderEncoder, VideoEncoder};
use capture::{Monitor, ScreenCapture, SynthCapture};
use landesk_core::error::CoreError;
use landesk_core::oneclick::OneClickSession;
use lp::datagram::{NaluKind, VideoDgram, FLAG_FIRST};

/// 发流参数（环回测试 / 生产默认值）。
pub struct PumpConfig {
    /// 编码器 kind（喂 [`lp::msg::EncodeParams`] 标签；占位后端恒 Openh264=9）。
    pub kind: u8,
    /// 目标帧率（泵按 1/fps 节拍出帧；环回测试取小值加快）。
    pub fps: u32,
}

impl Default for PumpConfig {
    fn default() -> Self {
        Self {
            kind: 9,
            fps: 30,
        }
    }
}

/// 一帧待发 NALU（编码产物，已含 Annex-B 起始码前缀）。
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// NALU 类型（I / P / SPSPPS）。
    pub kind: u8,
    /// 裸 H.264 NALU 字节（含起始码）。
    pub nalu: Vec<u8>,
}

/// 视频发流泵（P2②）：把「采集 → 编码 → NALU」产出一帧，经 [`VideoDgram::fragment_nalu`]
/// 分片后逐片 `OneClickSession::send_video` 发出。
///
/// 会话用 `&self` 的 `send_video`（`quinn::Connection` 共享 UDP 驱动，可并发调用），
/// 故泵不独占会话，可与其他控制帧并发。
pub struct VideoPump {
    cfg: PumpConfig,
    /// 合成画面源（无 OS 依赖；逐帧产真·变化像素，左上角烧录帧号）。
    capture: SynthCapture,
    /// 占位编码后端（Openh264 槽，1 帧进 = 1 NALU 出）。
    encoder: PlaceholderEncoder,
    /// 帧序号（喂 VideoDgram.seq，u16 循环空间）。
    seq: u16,
    /// 是否首帧（首帧强制 IDR + SPS/PPS，供对端解码器 flush + 重建参考帧）。
    first: bool,
}

impl VideoPump {
    /// 在 `w×h` 分辨率上起泵（fps 见 [`PumpConfig`]）。`open` 编码器，失败即报错。
    pub fn new(w: u32, h: u32, cfg: PumpConfig) -> Result<Self, String> {
        let monitor = Monitor {
            index: 0,
            width: w,
            height: h,
            name_hash: 0,
        };
        let mut encoder = PlaceholderEncoder::new();
        // 编码失败归一为可读消息（零分辨率等）。
        encoder.open(w, h, cfg.fps).map_err(|e| e.to_string())?;
        Ok(Self {
            cfg,
            capture: SynthCapture::default().with_monitor(monitor),
            encoder,
            seq: 0,
            first: true,
        })
    }

    /// 产出一帧 NALU（采集一帧 → 占位编码 → 取 NALU）。首帧标 IDR+SPS/PPS，其后标 P 帧。
    ///
    /// 占位后端 1 进 1 出：`send_frame` 推一帧入队、`recv_nalu` 出队（含 H.264 起始码前缀）。
    pub fn next_frame(&mut self) -> Result<VideoFrame, String> {
        let frame = self.capture.grab(0).map_err(|e| e.to_string())?;
        self.encoder.send_frame(&frame.data).map_err(|e| e.to_string())?;
        let nalu = self
            .encoder
            .recv_nalu()
            .ok_or_else(|| "no nalu after send_frame".to_string())?;
        let kind = if self.first {
            NaluKind::IdrWithSpsPps as u8
        } else {
            NaluKind::PFrame as u8
        };
        self.first = false;
        Ok(VideoFrame { kind, nalu })
    }

    /// 取帧节拍（1/fps；fps=0 不节流）。
    pub fn interval(&self) -> Option<Duration> {
        (self.cfg.fps > 0)
            .then(|| Duration::from_millis(1000 / self.cfg.fps.max(1) as u64))
    }

    /// 把一帧 NALU 分片后逐片发到会话（`send_video` 发**明文** `encode_plain` 输出）。
    ///
    /// 分片由 [`VideoDgram::fragment_nalu`] 完成（≤MTU），首片置 [`FLAG_FIRST`]（解码器
    /// flush 点），末片置末片位；接收端 [`lp::jitter::JitterBuffer`] 按 `seq`+分片号拼接还原。
    pub fn send(&mut self, session: &OneClickSession, vf: &VideoFrame) -> Result<u32, CoreError> {
        self.seq = self.seq.wrapping_add(1);
        let flags = if vf.kind == NaluKind::IdrWithSpsPps as u8 {
            FLAG_FIRST
        } else {
            0
        };
        let frags = VideoDgram::fragment_nalu(&vf.nalu, self.seq, vf.kind, flags);
        for f in &frags {
            // `encode_plain` 出 [`lp::LpError`]，经 `CoreError::Frame(#[from])` 直接转。
            let plain = f.encode_plain()?;
            session.send_video(&plain)?;
        }
        Ok(frags.len() as u32)
    }
}
