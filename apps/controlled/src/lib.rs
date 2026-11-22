//! 受控端视频发流泵（P2②，矩阵 §7.5「画面」，FR-03/FR-04）。
//!
//! 实现已下沉到 [`landesk_core::video_pump`]，供 CLI 与 GUI 受控端共用；本 crate
//! 仅 re-export 保持 `controlled::{VideoPump, PumpConfig, VideoFrame}` API 兼容。
//!
//! 数据面链路：采集（合成源，离线可验证；真实平台采集按期二替换）→ 占位编码
//! （Openh264 槽，真编码期三替换）→ `lp::datagram::VideoDgram::fragment_nalu` 切 ≤MTU
//! 分片 → QUIC 视频 Datagram。接收端走 `landesk-ui` 视频泵重组，见
//! `apps/controlled/tests/video_pump_loopback.rs` 端到端校验。

pub use landesk_core::video_pump::{PumpConfig, VideoFrame, VideoPump};
