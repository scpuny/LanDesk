//! 占位编码后端（FR-04 数据面填肉：打通「采集 → 编码 → VideoDgram」最薄真链路）。
//!
//! [`PlaceholderEncoder`] 占 **Openh264（`lp::msg::EncoderKind::Openh264` = 9）软编兜底槽位**：
//! 纯 Rust 实现，**不链接** ffmpeg / openh264，把送入的每一帧字节直接包成一条
//! 带 H.264 Annex-B 起始码（`00 00 00 01`）的裸 NALU（1 帧进 = 1 NALU 出）。
//! 目的不是真压缩，而是给数据面回环测试（C3）一个「有形态、可校验」的编码产物：
//! 采集帧 → 占位编码 → `lp::datagram::VideoDgram` → 真 QUIC Datagram → 对端还原 NALU，
//! 证明数据面通路在真实 QUIC 上跑通。
//!
//! 真实 openh264 / 硬编（qsv/nvenc/amf/v4l2m2m/videotoolbox）后端在后续里程碑
//! 按 feature（`openh264` 等）接入，届时本占位可替换/共存。

use std::collections::VecDeque;

use crate::{EncoderError, VideoEncoder};

/// Openh264 软编槽位 kind 值（与 `lp::msg::EncoderKind::Openh264` 一致）。
const OPENH264: u8 = 9;

/// H.264 Annex-B 起始码（NALU 前导，裸流拼接约定）。
pub const NALU_START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// 占位编码后端（Openh264 槽位，kind=9）。
///
/// `send_frame` 把入帧字节前缀 H.264 起始码后入队，`recv_nalu` 出队返回；
/// 未 open 时 `send_frame` 报错，`close` 清空队列并复位 open 状态。
#[derive(Debug, Default)]
pub struct PlaceholderEncoder {
    opened: bool,
    width: u32,
    height: u32,
    fps: u32,
    /// 已编码、待 `recv_nalu` 取出的 NALU（带起始码）。
    out: VecDeque<Vec<u8>>,
}

impl PlaceholderEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 最近一次 `open` 记录的分辨率与帧率（未 open 时全为 0），供可观测/断言。
    pub fn params(&self) -> (u32, u32, u32) {
        (self.width, self.height, self.fps)
    }

    /// 待取出的编码 NALU 数量。
    pub fn pending(&self) -> usize {
        self.out.len()
    }
}

impl VideoEncoder for PlaceholderEncoder {
    fn kind(&self) -> u8 {
        OPENH264
    }

    fn open(&mut self, width: u32, height: u32, fps: u32) -> Result<(), EncoderError> {
        if width == 0 || height == 0 {
            return Err(EncoderError::OpenFailed("resolution must be nonzero".into()));
        }
        self.opened = true;
        self.width = width;
        self.height = height;
        self.fps = fps;
        Ok(())
    }

    fn send_frame(&mut self, nalu_in: &[u8]) -> Result<(), EncoderError> {
        if !self.opened {
            return Err(EncoderError::OpenFailed("encoder not opened".into()));
        }
        // 占位「编码」：原帧字节前缀 H.264 起始码，作为一条裸 NALU。
        let mut nalu = Vec::with_capacity(NALU_START_CODE.len() + nalu_in.len());
        nalu.extend_from_slice(&NALU_START_CODE);
        nalu.extend_from_slice(nalu_in);
        self.out.push_back(nalu);
        Ok(())
    }

    fn recv_nalu(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }

    fn close(&mut self) {
        self.opened = false;
        self.out.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_openh264_slot() {
        let e = PlaceholderEncoder::new();
        assert_eq!(e.kind(), 9, "占位后端占 Openh264 软编槽位");
    }

    #[test]
    fn encode_produces_start_code_prefixed_nalu() {
        let mut e = PlaceholderEncoder::new();
        e.open(64, 48, 30).unwrap();
        assert_eq!(e.params(), (64, 48, 30), "open 记录分辨率/帧率");

        let frame = [0xAA, 0xBB, 0xCC];
        e.send_frame(&frame).unwrap();
        e.send_frame(&[0x11]).unwrap();

        let n1 = e.recv_nalu().expect("应有一条 NALU");
        let mut want = NALU_START_CODE.to_vec();
        want.extend_from_slice(&frame);
        assert_eq!(n1, want, "NALU = 起始码 ++ 原帧字节");

        let n2 = e.recv_nalu().expect("第二条 NALU");
        assert_eq!(&n2[0..4], &NALU_START_CODE, "第二帧同样带起始码");
        assert_eq!(&n2[4..], &[0x11]);

        assert!(e.recv_nalu().is_none(), "1 进 1 出，队空后返回 None");
    }

    #[test]
    fn send_before_open_fails() {
        let mut e = PlaceholderEncoder::new();
        let err = e.send_frame(&[0, 0, 0, 1]).unwrap_err();
        assert!(matches!(err, EncoderError::OpenFailed(_)), "未 open 送帧应报 OpenFailed");
    }

    #[test]
    fn open_zero_resolution_rejects() {
        let mut e = PlaceholderEncoder::new();
        assert!(e.open(0, 0, 30).is_err(), "零分辨率应 open 失败，供 factory 降级识别");
    }

    #[test]
    fn close_clears_and_resets() {
        let mut e = PlaceholderEncoder::new();
        e.open(16, 16, 30).unwrap();
        e.send_frame(&[0xDE, 0xAD]).unwrap();
        e.close();
        assert_eq!(e.pending(), 0, "close 清空待取 NALU");
        assert!(e.send_frame(&[0]).is_err(), "close 后须重新 open");
    }
}
