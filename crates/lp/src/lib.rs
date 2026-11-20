//! `lp` — LanDesk 协议库。
//!
//! 对齐 `docs/protocol.md`。设计约束：
//! - 纯逻辑、无 tokio、无平台 API，全部可单测（ADR-A1）；
//! - 字节序一律小端（LE）；
//! - 控制流帧结构：`ver(1) | type(1) | len(2LE) | payload | gcm_tag(12)`；
//! - 视频 Datagram 结构：`mag(2) | ver(1) | seq(2) | kind(1) | frag(1) | flags(1) | nalu_len(2) | nalu_data`，整包再经 GCM 加密。

pub mod wire;
pub mod msg;
pub mod datagram;
pub mod jitter;
pub mod clipboard;
pub mod keycodes;
pub mod error;

pub use error::LpError;

/// 协议版本（MVP 固定 1，对齐 protocol.md §0）。
pub const PROTO_VER: u8 = 1;
/// 控制帧载荷单帧上限（对齐 protocol.md §1.1）。
/// 帧内 `len` 字段为 u16（2B LE），故有效载荷上限即 `u16::MAX`（65535B）。
pub const CONTROL_MAX_PAYLOAD: usize = u16::MAX as usize;
/// 视频 Datagram 单包上限（1200B MTU - QUIC/UDP/IP 开销，对齐 protocol.md §2.1）。
pub const DGRAM_MAX: usize = 1232;
/// 视频 Datagram 魔数 "LD"（对齐 protocol.md §2.2）。
pub const DGRAM_MAGIC: u16 = 0x4C_44;
