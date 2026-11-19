//! 剪贴板分段/重组（对齐 `docs/protocol.md` §1.3，FR-07）。
//!
//! 控制流上的分段协议：`ClipStart(total_len) → ClipChunk(seq, data)* → ClipEnd(sha256)`。
//! 单份剪贴板上限 1MB，超出报 [`LpError::ClipboardTooLarge`]。

use sha2::{Digest, Sha256};

use crate::error::LpError;
use crate::msg::{CLIP_TOTAL_CAP, CLIP_CHUNK_CAP, ClipboardChunk, ClipboardEnd, ClipboardStart};

/// 按 [`CLIP_CHUNK_CAP`] 切分文本，一次生成全部分段（≤1MB，分段 ≤33 个，可接受）。
pub fn split_clipboard(data: &[u8]) -> Result<Vec<ClipboardChunk>, LpError> {
    if data.len() > CLIP_TOTAL_CAP as usize {
        return Err(LpError::ClipboardTooLarge(data.len()));
    }
    let mut out = Vec::new();
    let mut seq = 0u16;
    let mut off = 0usize;
    while off < data.len() {
        let end = std::cmp::min(off + CLIP_CHUNK_CAP, data.len());
        out.push(ClipboardChunk {
            seq,
            data: data[off..end].to_vec(),
        });
        off = end;
        seq += 1;
    }
    Ok(out)
}

/// 无预分配的单段生成（生产路径推荐，供 landesk-core 发送循环逐段取用）。
pub fn chunk_at(data: &[u8], seq: u16) -> Option<ClipboardChunk> {
    let off = (seq as usize) * CLIP_CHUNK_CAP;
    if off >= data.len() {
        return None;
    }
    let end = std::cmp::min(off + CLIP_CHUNK_CAP, data.len());
    Some(ClipboardChunk {
        seq,
        data: data[off..end].to_vec(),
    })
}

/// 接收端重组状态机。
#[derive(Debug, Default)]
pub struct ClipboardReassembler {
    total_len: u32,
    buffer: Vec<u8>,
    next_seq: u16,
    sha: Option<[u8; 32]>,
}

impl ClipboardReassembler {
    pub fn on_start(&mut self, start: &ClipboardStart) -> Result<(), LpError> {
        if start.total_len > CLIP_TOTAL_CAP {
            return Err(LpError::ClipboardTooLarge(start.total_len as usize));
        }
        self.total_len = start.total_len;
        self.buffer.clear();
        self.next_seq = 0;
        self.sha = None;
        Ok(())
    }

    /// 返回 `true` 表示全部收齐，可取文本。
    pub fn on_chunk(&mut self, chunk: &ClipboardChunk) -> Result<bool, LpError> {
        if chunk.seq != self.next_seq {
            // 乱序分段：M1 直接丢弃并要求对端重发（控制流可靠，重发代价可接受）
            return Err(LpError::Reassembly(format!(
                "clipboard chunk out of order: got {}, expected {}",
                chunk.seq, self.next_seq
            )));
        }
        self.buffer.extend_from_slice(&chunk.data);
        self.next_seq += 1;
        if self.buffer.len() > self.total_len as usize {
            return Err(LpError::ClipboardTooLarge(self.buffer.len()));
        }
        Ok(self.buffer.len() == self.total_len as usize)
    }

    /// 校验 SHA-256 后取回文本。
    pub fn on_end(&mut self, end: &ClipboardEnd) -> Result<String, LpError> {
        if self.sha.is_none() && self.next_seq > 0 {
            let mut hasher = Sha256::new();
            hasher.update(&self.buffer);
            // 显式标注目标类型为 `[u8; 32]`，消除 `.into()` 的类型推断歧义（E0283）。
            let digest: [u8; 32] = hasher.finalize().into();
            if digest != end.sha256 {
                return Err(LpError::Reassembly("clipboard sha256 mismatch".into()));
            }
            self.sha = Some(end.sha256);
        }
        if self.buffer.len() != self.total_len as usize {
            return Err(LpError::Reassembly("clipboard incomplete".into()));
        }
        String::from_utf8(self.buffer.clone()).map_err(|_| LpError::Reassembly("clipboard not utf8".into()))
    }

    pub fn digest_of(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_chunks() {
        let text: &[u8] = b"hello landesk clipboard";
        let start = ClipboardStart {
            total_len: text.len() as u32,
            mime_flags: 0,
        };
        let mut rx = ClipboardReassembler::default();
        rx.on_start(&start).unwrap();
        for chunk in split_clipboard(text).unwrap() {
            rx.on_chunk(&chunk).unwrap();
        }
        let end = ClipboardEnd {
            sha256: ClipboardReassembler::digest_of(text),
        };
        assert_eq!(rx.on_end(&end).unwrap(), "hello landesk clipboard");
    }

    #[test]
    fn oversized_rejected() {
        let big = vec![0u8; CLIP_TOTAL_CAP as usize + 1];
        assert!(split_clipboard(&big).is_err());
        let start = ClipboardStart {
            total_len: CLIP_TOTAL_CAP + 1,
            mime_flags: 0,
        };
        let mut rx = ClipboardReassembler::default();
        assert!(rx.on_start(&start).is_err());
    }
}
