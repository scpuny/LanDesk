//! `avcodec-wrap` — 统一编码器封装（FR-04，核心稳定模块）。
//!
//! M1/M2 阶段目标：先落**降级链 + 异常重置状态机**（纯逻辑可单测，ADR-A1 风险前移），
//! libav FFI 各编码器后端（qsv/nvenc/amf/v4l2m2m/videotoolbox）与 OpenH264 软编
//! 按 feature 门控，后续里程碑补全。

pub mod backends;
pub mod factory;
pub mod reset;
#[cfg(feature = "openh264")]
pub mod openh264_backend;

use thiserror::Error;

pub use backends::{NALU_START_CODE, PlaceholderEncoder};
#[cfg(feature = "openh264")]
pub use openh264_backend::Openh264Encoder;

/// 编码器错误（对齐 NFR-01：驱动异常时降级，不崩溃）。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncoderError {
    #[error("encoder open failed: {0}")]
    OpenFailed(String),
    #[error("encoder unavailable on this platform")]
    Unavailable,
    #[error("frame encode failed: {0}")]
    EncodeFailed(String),
    #[error("encoder reset required")]
    ResetRequired,
}

/// 统一编码器 Trait（FR-04.1：open / send_frame / recv_nalu / close）。
pub trait VideoEncoder: Send {
    /// 编码器标识（`lp::msg::EncoderKind` 的 u8 值）。
    fn kind(&self) -> u8;

    /// 打开编码器（失败应返回 `Err`，由 [`factory::EncoderFactory`] 触发降级）。
    fn open(&mut self, width: u32, height: u32, fps: u32) -> Result<(), EncoderError>;

    /// 送入一帧（NV12）。
    fn send_frame(&mut self, nalu_in: &[u8]) -> Result<(), EncoderError>;

    /// 取出一条编码出的裸 H.264 输出单元（FR-04.3 不用容器）。
    ///
    /// 占位后端返回单条 NALU；真编码后端（openh264）返回**一整个 Annex-B 访问单元**
    /// ——可能含 SPS+PPS+IDR 多条 NALU，WebCodecs 以单个 `EncodedVideoChunk` 解码。
    fn recv_nalu(&mut self) -> Option<Vec<u8>>;

    /// 最近一次 `send_frame` 产出的访问单元是否含 IDR 关键帧。
    ///
    /// 默认 `false`：占位后端非真 H.264，关键帧语义由帧序（首帧）决定；
    /// 真编码后端覆写此方法，把 openh264 的周期性 IDR 正确标给对端解码器。
    fn is_keyframe(&self) -> bool {
        false
    }

    /// 关闭（异常时由 reset 模块驱动重开）。
    fn close(&mut self);
}

/// 编码器能力描述（供 factory 按平台 caps 排序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderCaps {
    /// 平台可用的编码器优先级链（首位最优先，末位兜底）。
    pub priority: [u8; 6],
    pub count: u8,
}

impl EncoderCaps {
    /// 取前 `count` 个有效编码器。
    pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
        self.priority[..self.count as usize].iter().copied()
    }
}
