//! `avcodec-wrap` — 统一编码器封装（FR-04，核心稳定模块）。
//!
//! M1/M2 阶段目标：先落**降级链 + 异常重置状态机**（纯逻辑可单测，ADR-A1 风险前移），
//! libav FFI 各编码器后端（qsv/nvenc/amf/v4l2m2m/videotoolbox）与 OpenH264 软编
//! 按 feature 门控，后续里程碑补全。

pub mod factory;
pub mod reset;

use thiserror::Error;

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

    /// 取出一条编码出的裸 H.264 NALU（FR-04.3 不用容器）。
    fn recv_nalu(&mut self) -> Option<Vec<u8>>;

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
