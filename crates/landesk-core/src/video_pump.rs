//! 受控端视频数据面发流（P2② → 受控闭环，FR-03/FR-04）。
//!
//! 打通「采集 → 编码 → NALU 分片 → QUIC 视频 Datagram」发流泵：
//! - 采集默认用 [`capture::SynthCapture`]（逐帧产真·变化像素，左上角烧录帧号，
//!   无 OS 依赖、离线可验证）；真实平台采集在期二按 feature 替换；
//! - 编码默认用 [`avcodec_wrap::PlaceholderEncoder`]（Openh264 软编兜底槽，
//!   纯 Rust 不链接 ffmpeg）；真 H.264 编码在期三替换；
//! - NALU 经 [`lp::datagram::VideoDgram::fragment_nalu`] 切成 ≤MTU 分片，逐片发出。
//!
//! 本模块从 `apps/controlled` 下沉到 `landesk-core`，使 GUI（受控端）与 CLI 共用
//! 同一发流实现。[`VideoPump::send`] 兼容旧 CLI（`&OneClickSession`）；
//! [`VideoPump::send_tx`] 用独立 [`VideoDatagramTx`] 句柄，供 GUI 受控端把发流泵
//! `move` 进独立任务，与键鼠控制帧接收并发（不争用会话控制流 `&mut`）。

use std::time::Duration;

use avcodec_wrap::{PlaceholderEncoder, VideoEncoder};
use capture::{Monitor, ScreenCapture, SynthCapture};
use quic_link::VideoDatagramTx;

use crate::error::CoreError;
use crate::oneclick::OneClickSession;
use lp::datagram::{NaluKind, VideoDgram, FLAG_FIRST};

/// 发流参数（环回测试 / 生产默认值）。
#[derive(Debug, Clone, Copy)]
pub struct PumpConfig {
    /// 编码器 kind（喂 [`lp::msg::EncodeParams`] 标签；占位后端恒 Openh264=9）。
    pub kind: u8,
    /// 目标帧率（泵按 1/fps 节拍出帧；环回测试取小值加快）。
    pub fps: u32,
}

impl Default for PumpConfig {
    fn default() -> Self {
        Self { kind: 9, fps: 30 }
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

/// 视频发流泵：把「采集 → 编码 → NALU」产出的一帧分片后逐片发出。
///
/// 会话视频 Datagram 用 `&self` 发送（quinn 共享 UDP 驱动），故泵不独占会话，
/// 可与其他控制帧并发。
pub struct VideoPump {
    cfg: PumpConfig,
    /// 画面源：优先平台真实采屏，失败降级合成源（无 OS 依赖）。
    capture: Box<dyn ScreenCapture>,
    /// 占位编码后端（Openh264 槽，1 帧进 = 1 NALU 出；期三换真编码）。
    encoder: PlaceholderEncoder,
    /// 实际采集分辨率（真实采屏取显示器尺寸，合成源取请求值）。
    width: u32,
    height: u32,
    /// 帧序号（喂 VideoDgram.seq，u16 循环空间）。
    seq: u16,
    /// 是否首帧（首帧强制 IDR + SPS/PPS，供对端解码器 flush + 重建参考帧）。
    first: bool,
}

impl VideoPump {
    /// 在 `w×h` 分辨率上起泵（fps 见 [`PumpConfig`]），使用合成源。
    ///
    /// 占位编码阶段（期一/期二）不压缩，真实采屏的全屏帧会产生数 MB 数据、分片
    /// 超出 QUIC datagram 容量而丢包——故默认用轻量合成源验证链路。期三真编码
    /// 接入后由 [`Self::new_with_capture`] 切到平台真实采屏。
    pub fn new(w: u32, h: u32, cfg: PumpConfig) -> Result<Self, String> {
        let monitor = Monitor {
            index: 0,
            width: w,
            height: h,
            name_hash: 0,
        };
        Self::new_with_capture(Box::new(SynthCapture::default().with_monitor(monitor)), w, h, cfg)
    }

    /// 用指定采集器起泵（真实采屏接入点，期三真编码后启用）。
    ///
    /// 采集器分辨率由调用方按 `monitors()` 传入（通常与 w×h 一致）；编码器同分辨率打开。
    pub fn new_with_capture(
        capture: Box<dyn ScreenCapture>,
        w: u32,
        h: u32,
        cfg: PumpConfig,
    ) -> Result<Self, String> {
        let mut encoder = PlaceholderEncoder::new();
        encoder.open(w, h, cfg.fps).map_err(|e| e.to_string())?;
        Ok(Self {
            cfg,
            capture,
            encoder,
            width: w,
            height: h,
            seq: 0,
            first: true,
        })
    }

    /// 当前实际采集分辨率。
    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 产出一帧 NALU（采集一帧 → 占位编码 → 取 NALU）。首帧标 IDR+SPS/PPS，其后标 P 帧。
    ///
    /// 真实采屏在某帧失败（如屏幕录制权限被撤销、显示器休眠）→ 永久降级为同分辨率
    /// 合成源，保证发流不中断（画面质量降级但会话仍在）。
    pub fn next_frame(&mut self) -> Result<VideoFrame, String> {
        let frame = match self.capture.grab(0) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "真实采屏失败，降级为合成源");
                let monitor = Monitor {
                    index: 0,
                    width: self.width,
                    height: self.height,
                    name_hash: 0,
                };
                self.capture = Box::new(SynthCapture::default().with_monitor(monitor));
                self.capture.grab(0).map_err(|e| e.to_string())?
            }
        };
        self.encoder
            .send_frame(&frame.data)
            .map_err(|e| e.to_string())?;
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
        (self.cfg.fps > 0).then(|| Duration::from_millis(1000 / self.cfg.fps.max(1) as u64))
    }

    /// 把一帧 NALU 分片（首片置 FLAG_FIRST）。递增 seq，返回分片列表。
    fn fragment(&mut self, vf: &VideoFrame) -> Vec<VideoDgram> {
        self.seq = self.seq.wrapping_add(1);
        let flags = if vf.kind == NaluKind::IdrWithSpsPps as u8 {
            FLAG_FIRST
        } else {
            0
        };
        VideoDgram::fragment_nalu(&vf.nalu, self.seq, vf.kind, flags)
    }

    /// 分片后经 `OneClickSession::send_video` 发出（CLI / 回环测试兼容入口）。
    pub fn send(&mut self, session: &OneClickSession, vf: &VideoFrame) -> Result<u32, CoreError> {
        let frags = self.fragment(vf);
        let n = frags.len() as u32;
        for f in &frags {
            session.send_video(&f.encode_plain()?)?;
        }
        Ok(n)
    }

    /// 分片后经独立 [`VideoDatagramTx`] 发出（GUI 受控端发流泵入口）。
    ///
    /// 与 [`Self::send`] 等价，但只持有克隆出的视频发送句柄，不占会话控制流
    /// `&mut`，可与键鼠控制帧接收循环并发。
    pub fn send_tx(&mut self, tx: &VideoDatagramTx, vf: &VideoFrame) -> Result<u32, CoreError> {
        let frags = self.fragment(vf);
        let n = frags.len() as u32;
        for f in &frags {
            tx.send(&f.encode_plain()?)?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_is_idr_then_p() {
        let mut p = VideoPump::new(32, 24, PumpConfig { kind: 9, fps: 0 }).expect("pump");
        assert_eq!(
            p.next_frame().expect("f0").kind,
            NaluKind::IdrWithSpsPps as u8
        );
        assert_eq!(p.next_frame().expect("f1").kind, NaluKind::PFrame as u8);
        assert_eq!(p.interval(), None, "fps=0 不节流");
    }
}
