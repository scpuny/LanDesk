//! 视频 Datagram 编解码（对齐 `docs/protocol.md` §2.2）。
//!
//! 明文报文结构（整包再经 GCM 加密，nonce 按 §0.1 Datagram 规则）：
//! ```text
//! +----+---+------+---+-----+-------+----------+-------------+
//! |mag 2|ver|seq 2|kind|frag |flags 1|nalu_len 2|nalu_data   |
//! +----+---+------+---+-----+-------+----------+-------------+
//! ```
//! `frag` 高 7 bit = 分片号，bit7 = 末片标志。

use crate::error::LpError;
use crate::{DGRAM_MAGIC, PROTO_VER};

/// NALU 类型（对齐 protocol.md §2.2 kind 字段）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NaluKind {
    /// I 帧 + 附带 SPS/PPS。
    IdrWithSpsPps = 0,
    /// I 帧。
    Idr = 1,
    /// P 帧。
    PFrame = 2,
    /// SPS/PPS 单独重传（分辨率变更/重连后强制重发）。
    SpsPps = 3,
}

impl NaluKind {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => NaluKind::IdrWithSpsPps,
            1 => NaluKind::Idr,
            2 => NaluKind::PFrame,
            3 => NaluKind::SpsPps,
            _ => return None,
        })
    }
}

/// flags 位定义（对齐 protocol.md §2.2）。
pub const FLAG_FIRST: u8 = 0b01; // 解码器 flush 用
pub const FLAG_STALE_DROP: u8 = 0b10; // 接收端标记：过期帧可丢

/// 视频 Datagram 明文体。
///
/// 注意：含变长 `nalu`，只能 `Clone` 不可 `Copy`（按引用传递或显式 `clone()`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoDgram {
    pub seq: u16,
    pub kind: u8,
    /// 高 7 bit = 分片号；bit7 = 末片。
    pub frag: u8,
    pub flags: u8,
    /// 本 Datagram 携带的 NALU 字节（单包内为 NALU 片段）。
    pub nalu: Vec<u8>,
}

impl VideoDgram {
    pub const FRAG_LAST_BIT: u8 = 0x80;

    pub const fn frag_no(&self) -> u8 {
        self.frag & !Self::FRAG_LAST_BIT
    }

    pub const fn is_last(&self) -> bool {
        self.frag & Self::FRAG_LAST_BIT != 0
    }

    pub fn is_idr(&self) -> bool {
        self.kind == NaluKind::Idr as u8 || self.kind == NaluKind::IdrWithSpsPps as u8
    }

    /// 明文序列化（不含 GCM 层）。
    pub fn encode_plain(&self) -> Result<Vec<u8>, LpError> {
        if self.nalu.len() > crate::DGRAM_MAX {
            return Err(LpError::Reassembly(format!(
                "nalu fragment {} bytes exceeds dgram cap {}",
                self.nalu.len(),
                crate::DGRAM_MAX
            )));
        }
        let mut buf = Vec::with_capacity(9 + self.nalu.len());
        buf.extend_from_slice(&DGRAM_MAGIC.to_le_bytes());
        buf.push(PROTO_VER);
        buf.extend_from_slice(&self.seq.to_le_bytes());
        buf.push(self.kind);
        buf.push(self.frag);
        buf.push(self.flags);
        buf.extend_from_slice(&(self.nalu.len() as u16).to_le_bytes());
        buf.extend_from_slice(&self.nalu);
        Ok(buf)
    }

    /// 反序列化明文报文，校验魔数与长度。
    pub fn decode_plain(buf: &[u8]) -> Result<Self, LpError> {
        // 头部：mag 2 + ver 1 + seq 2 + kind 1 + frag 1 + flags 1 + nalu_len 2 = 10B
        const HEADER: usize = 10;
        if buf.len() < HEADER {
            return Err(LpError::BufferTooShort { len: buf.len(), need: HEADER });
        }
        let magic = u16::from_le_bytes([buf[0], buf[1]]);
        if magic != DGRAM_MAGIC {
            return Err(LpError::Reassembly(format!(
                "bad magic 0x{magic:04x}, not a landesk video dgram"
            )));
        }
        if buf[2] != PROTO_VER {
            return Err(LpError::BadVersion(buf[2]));
        }
        let seq = u16::from_le_bytes([buf[3], buf[4]]);
        let kind = buf[5];
        if NaluKind::from_u8(kind).is_none() {
            return Err(LpError::Reassembly(format!("unknown nalu kind {kind}")));
        }
        let frag = buf[6];
        let flags = buf[7];
        let nalu_len = u16::from_le_bytes([buf[8], buf[9]]) as usize;
        let need = HEADER + nalu_len;
        if buf.len() < need {
            return Err(LpError::BufferTooShort { len: buf.len(), need });
        }
        let nalu = buf[HEADER..need].to_vec();
        Ok(Self { seq, kind, frag, flags, nalu })
    }
}
