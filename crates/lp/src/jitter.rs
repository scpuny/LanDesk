//! 视频 Datagram 分片重组 + 接收端抖动缓冲（对齐 `docs/protocol.md` §2.3）。
//!
//! 设计约束（M1/M2 阶段纯逻辑实现，后续 M3 接入 QUIC）：
//! - 单 NALU 跨包按 `frag` 顺序拼接，重组超时 200ms 丢弃（本层用帧计数近似）；
//! - 环形缓冲 8 帧 / 200ms 上限；
//! - `seq` 落后当前最大 seq 的 P 帧直接丢弃；IDR 必保留；
//! - 缓冲满时丢最旧 P 帧。

use crate::datagram::VideoDgram;

/// 帧序号循环空间（u16 用 65535 循环）。
const SEQ_MOD: i32 = 65536;

/// 环形缓冲帧数上限（对齐 protocol.md §2.3）。
const RING_CAP: usize = 8;

/// 一帧（已按 frag 拼接完成的 NALU 单元）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub seq: u16,
    pub kind: u8,
    pub flags: u8,
    pub nalu: Vec<u8>,
}

impl Frame {
    pub fn is_idr(&self) -> bool {
        use crate::datagram::NaluKind;
        self.kind == NaluKind::Idr as u8 || self.kind == NaluKind::IdrWithSpsPps as u8
    }
}

/// 接收端 NALU 分片重组 + 抖动缓冲。
#[derive(Debug, Default)]
pub struct JitterBuffer {
    ring: std::collections::VecDeque<Frame>,
    /// 当前正在重组的 NALU（按 seq + frag）。
    reassembling: Option<PartialFrame>,
}

#[derive(Debug, Default)]
struct PartialFrame {
    seq: u16,
    kind: u8,
    flags: u8,
    /// 各分片数据，下标 = frag 号（末包到达前用 Option 标记缺口）。
    parts: Vec<Option<Vec<u8>>>,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 投入一个视频 Datagram，返回本步产生的完整帧（0 或 1 帧）。
    /// 产生的完整帧同时写入抖动缓冲（除被丢弃的过期 P 帧）。
    pub fn push(&mut self, d: &VideoDgram) -> Option<Frame> {
        let mut frame: Option<Frame> = None;

        if d.is_last() {
            if d.frag_no() == 0 {
                // 单包完整帧
                frame = Some(Frame {
                    seq: d.seq,
                    kind: d.kind,
                    flags: d.flags,
                    nalu: d.nalu.clone(),
                });
                self.reassembling = None;
            } else if let Some(mut pf) = self.reassembling.take() {
                if pf.seq == d.seq {
                    // 拼接：中间分片 + 末片自身数据
                    let mut nalu = Vec::new();
                    for p in pf.parts.drain(..) {
                        match p {
                            Some(chunk) => nalu.extend(chunk),
                            None => {
                                // 缺中间片：整 NALU 丢弃
                                return None;
                            }
                        }
                    }
                    nalu.extend(d.nalu.clone());
                    frame = Some(Frame {
                        seq: pf.seq,
                        kind: pf.kind,
                        flags: pf.flags,
                        nalu,
                    });
                }
                // seq 不匹配：旧重组作废（前面分片全丢），末片无法独立成帧
            }
        } else {
            // 中间分片：累积
            if self.reassembling.as_ref().is_none_or(|pf| pf.seq != d.seq) {
                self.reassembling = Some(PartialFrame {
                    seq: d.seq,
                    kind: d.kind,
                    flags: d.flags,
                    parts: Vec::new(),
                });
            }
            let pf = self.reassembling.as_mut().unwrap();
            pf.parts.resize_with(d.frag_no() as usize + 1, Default::default);
            pf.parts[d.frag_no() as usize] = Some(d.nalu.clone());
        }

        // 抖动缓冲：乱序丢弃过期 P 帧；IDR 必保留
        if let Some(f) = frame.as_ref() {
            if !f.is_idr() {
                if let Some(max_seq) = self.ring.back().map(|fr| fr.seq) {
                    if seq_behind(f.seq, max_seq) {
                        return None; // 过期 P 帧丢弃
                    }
                }
            }
            // 缓冲满：丢最旧非 IDR 帧
            while self.ring.len() >= RING_CAP {
                if self.ring[0].is_idr() {
                    break;
                }
                self.ring.pop_front();
            }
        }
        // 将完整帧的克隆入抖动缓冲，原 `frame` 保留并返回给调用方
        // （`Option::take` 会消费值导致返回 None，故改用 `clone`）。
        if let Some(f) = frame.clone() {
            self.ring.push_back(f);
        }
        frame
    }

    /// 取出当前最早可解码帧（解码器按序消费）。
    pub fn pop(&mut self) -> Option<Frame> {
        self.ring.pop_front()
    }

    /// 缓冲中帧数。
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// 缓冲是否为空。
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// 清空（重连 / 强制 IDR 时调用）。
    pub fn clear(&mut self) {
        self.ring.clear();
        self.reassembling = None;
    }
}

/// `a` 是否在 `b` 之前（u16 循环语义，距离 < 半周期视为落后）。
fn seq_behind(a: u16, b: u16) -> bool {
    let diff = (a as i32 - b as i32 + SEQ_MOD) % SEQ_MOD;
    diff > SEQ_MOD / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(seq: u16, kind: u8, nalu: &[u8]) -> VideoDgram {
        // 单包完整帧：frag_no=0 且末片标志置位
        VideoDgram {
            seq,
            kind,
            frag: 0x80,
            flags: 0,
            nalu: nalu.to_vec(),
        }
    }

    #[test]
    fn out_of_order_p_dropped_idr_kept() {
        let mut jb = JitterBuffer::new();
        assert!(jb.push(&single(1, 2, b"I1")).is_some()); // P 帧入缓冲
        assert!(jb.push(&single(3, 2, b"I3")).is_some());
        // seq=2 的 P 帧晚到，落后于最大 seq=3 → 丢弃
        assert!(jb.push(&single(2, 2, b"I2")).is_none());
        assert_eq!(jb.len(), 2);
        // IDR 永远保留
        assert!(jb.push(&single(4, 1, b"IDR4")).is_some());
    }

    #[test]
    fn reassembles_fragments() {
        let mut jb = JitterBuffer::new();
        // 中间分片 frag_no=0（非末片）
        let d0 = VideoDgram {
            seq: 10,
            kind: 2,
            frag: 0,
            flags: 0,
            nalu: b"AA".to_vec(),
        };
        // 末片 frag_no=1 + 末片标志
        let d1 = VideoDgram {
            seq: 10,
            kind: 2,
            frag: 0x81,
            flags: 0,
            nalu: b"BB".to_vec(),
        };
        assert!(jb.push(&d0).is_none()); // 中间分片尚未成帧
        let f = jb.push(&d1).expect("last fragment completes frame");
        assert_eq!(f.nalu, b"AABB");
        assert_eq!(f.seq, 10);
    }

    #[test]
    fn missing_middle_fragment_drops_nalu() {
        let mut jb = JitterBuffer::new();
        // 只有 frag_no=1，缺 frag_no=0
        let d_missing = VideoDgram {
            seq: 20,
            kind: 2,
            frag: 1,
            flags: 0,
            nalu: b"X".to_vec(),
        };
        // 中间分片累积
        assert!(jb.push(&d_missing).is_none());
        let last = VideoDgram {
            seq: 20,
            kind: 2,
            frag: 0x82,
            flags: 0,
            nalu: b"Y".to_vec(),
        };
        // 末片到达但缺 frag_no=1 → 整 NALU 丢弃
        assert!(jb.push(&last).is_none());
        assert_eq!(jb.len(), 0);
    }
}
