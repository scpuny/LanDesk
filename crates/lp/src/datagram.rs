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

    /// 单包 NALU 载荷上限（`DGRAM_MAX` 减 10B 报头）。NALU 不超过此值走单包，否则分片。
    pub const FRAG_PAYLOAD_MAX: usize = crate::DGRAM_MAX - 10;

    /// 把一条完整 NALU 切分为可发送的 [`VideoDgram`] 分片序列（对齐 protocol.md §2.2）。
    ///
    /// - 单片（NALU 载荷 ≤ [`Self::FRAG_PAYLOAD_MAX`]）→ 一个 `frag=0x80`（分片号 0 + 末片）包；
    /// - 多片 → 高 7 bit 为递增分片号，末片置 bit7；首片额外置 [`FLAG_FIRST`]（解码器 flush 点）。
    ///
    /// 接收端 [`crate::jitter::JitterBuffer`] 按 `seq` + 分片号拼接还原整条 NALU。
    /// 返回的分片可直接 `encode_plain` 后经 QUIC Datagram 发送。
    pub fn fragment_nalu(nalu: &[u8], seq: u16, kind: u8, flags: u8) -> Vec<VideoDgram> {
        if nalu.len() <= Self::FRAG_PAYLOAD_MAX {
            return vec![VideoDgram {
                seq,
                kind,
                frag: Self::FRAG_LAST_BIT,
                flags,
                nalu: nalu.to_vec(),
            }];
        }
        let total = nalu.len();
        let count = total.div_ceil(Self::FRAG_PAYLOAD_MAX);
        (0..count)
            .map(|i| {
                let off = i * Self::FRAG_PAYLOAD_MAX;
                let end = (off + Self::FRAG_PAYLOAD_MAX).min(total);
                let is_last = i + 1 == count;
                let frag = (i as u8) | ((is_last as u8) * Self::FRAG_LAST_BIT);
                let f = if i == 0 { flags | FLAG_FIRST } else { flags };
                VideoDgram {
                    seq,
                    kind,
                    frag,
                    flags: f,
                    nalu: nalu[off..end].to_vec(),
                }
            })
            .collect()
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jitter::JitterBuffer;

    #[test]
    fn fragment_single_packet_for_small_nalu() {
        let nalu: Vec<u8> = (0..100).collect();
        let frags = VideoDgram::fragment_nalu(&nalu, 7, NaluKind::PFrame as u8, 0);
        assert_eq!(frags.len(), 1, "100B < FRAG_PAYLOAD_MAX 走单包");
        assert_eq!(frags[0].frag, VideoDgram::FRAG_LAST_BIT, "单包即末片（分片号 0）");
        assert!(frags[0].is_last(), "单包末片位置起");
        assert_eq!(frags[0].seq, 7);
        assert_eq!(frags[0].nalu, nalu);
    }

    #[test]
    fn fragment_split_and_reassemble() {
        // 3000B NALU > 1222B/片 → ceil(3000/1222)=3 片（1222+1222+556）。
        let nalu: Vec<u8> = (0..3000usize).map(|i| (i % 256) as u8).collect();
        let frags = VideoDgram::fragment_nalu(&nalu, 42, NaluKind::Idr as u8, 0);
        assert_eq!(frags.len(), 3, "3000B 应切 3 片");
        // 首片带 FLAG_FIRST（解码器 flush 点）；末片置 bit7。
        assert_eq!(frags[0].flags, FLAG_FIRST, "首片置 FLAG_FIRST");
        assert!(!frags[1].is_last(), "非末片 bit7=0");
        assert!(frags[2].is_last(), "末片 bit7=1");
        // 喂进 JitterBuffer 逐片拼接，末片到达时还原整条 NALU。
        let mut jb = JitterBuffer::new();
        assert!(jb.push(&frags[0]).is_none(), "中间分片不成帧");
        assert!(jb.push(&frags[1]).is_none(), "中间分片不成帧");
        let f = jb.push(&frags[2]).expect("末片到达应成帧");
        assert_eq!(f.nalu, nalu, "重组 NALU 与原始一致");
        assert_eq!(f.seq, 42);
        assert!(f.is_idr(), "kind=Idr 保留");
    }

    #[test]
    fn fragment_edge_exact_capacity() {
        // 恰好 FRAG_PAYLOAD_MAX（= DGRAM_MAX−10）→ 单片。
        let nalu = vec![0u8; VideoDgram::FRAG_PAYLOAD_MAX];
        assert_eq!(VideoDgram::fragment_nalu(&nalu, 1, 2, 0).len(), 1);
        // +1B → 2 片。
        let nalu = vec![0u8; VideoDgram::FRAG_PAYLOAD_MAX + 1];
        assert_eq!(VideoDgram::fragment_nalu(&nalu, 1, 2, 0).len(), 2);
    }

    /// 回归护栏：满片编码长度（= [`crate::DGRAM_MAX`]）必须 ≤ quinn 默认配置的**最坏可发
    /// datagram 下限**，否则 `send_datagram` 报 `TooLarge` → 发流泵首帧即停 → 远程画面黑屏。
    ///
    /// quinn 0.11 `TransportConfig::default()` 把 `initial_mtu`/`min_mtu` 都钉在
    /// `INITIAL_MTU=1200`；每包可发上限 = `min_mtu − predict_1rtt_overhead − Datagram::SIZE_BOUND`。
    /// 取最坏 1rtt 开销 32（flag 1 + CID 8 + PN 4 + 1-RTT tag 16 + 余量）与 `SIZE_BOUND=9`，
    /// 下限 = `1200 − 32 − 9 = 1159`。本断言守住 `DGRAM_MAX` 永远不越回 1232 这类不可发值。
    #[test]
    fn dgram_max_stays_within_quinn_guaranteed_floor() {
        const QUINN_GUARANTEED_FLOOR: usize = 1200 - 32 - 9; // = 1159，最坏可发下限
        // 编译期护栏：`DGRAM_MAX` 是 const，越回 1232 这类不可发值时这里直接编译失败。
        // const 块内不能用带格式化参数的 assert!（非 const 宏），故用静态消息。
        const {
            assert!(
                crate::DGRAM_MAX <= QUINN_GUARANTEED_FLOOR,
                "DGRAM_MAX 超 quinn 最坏可发下限，满片将 TooLarge → 发流黑屏"
            );
        }
        // 运行时长度校验：满片实际编码长度（10B 报头 + FRAG_PAYLOAD_MAX）也必须落在下限内。
        let full = VideoDgram {
            seq: 0,
            kind: 2,
            frag: VideoDgram::FRAG_LAST_BIT,
            flags: 0,
            nalu: vec![0u8; VideoDgram::FRAG_PAYLOAD_MAX],
        };
        let enc = full.encode_plain().expect("满片可编码");
        assert!(
            enc.len() <= QUINN_GUARANTEED_FLOOR,
            "满片编码 {}B 超 quinn 可发下限",
            enc.len()
        );
    }
}
