//! 视频数据面推流模型（P2②，矩阵 §7.5「画面」）：把接收端重组好的完整 NALU
//! （[`lp::jitter::Frame`]）归一成可跨宿主传递的序列化模型 [`VideoFrameMsg`]，
//! 供 `apps/ui-server`（SSE/流式端点）与 `ui/src-tauri`（通道命令）把画面帧推到前端。
//!
//! NALU 是二进制字节，宿主出参走 JSON（Tauri invoke / REST），故 [`VideoFrameMsg.nalu`]
//! 用 **base64** 字符串承载（前端 `atob` 还原喂 WebCodecs `VideoDecoder`）。本模块**不引
//! 新 crate**：内联一个极简 base64 编码器（标准字母表 + `=` 填充），保持离线 vendor 门纯净。

use lp::datagram::capture_degraded_from_flags;
use lp::jitter::Frame;
use serde::Serialize;

/// base64 标准字母表（RFC 4648）。
const B64: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 把任意字节序列编码为 base64 字符串（标准字母表，`=` 填充到 4 字节对齐）。
///
/// 纯本地逻辑、零依赖（离线门），供 NALU 字节跨 JSON 宿主传递。
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 2 < data.len() {
        // 每 3 字节 → 4 个 base64 字符。
        let n = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
        out.push(B64[(n >> 18 & 0x3F) as usize] as char);
        out.push(B64[(n >> 12 & 0x3F) as usize] as char);
        out.push(B64[(n >> 6 & 0x3F) as usize] as char);
        out.push(B64[(n & 0x3F) as usize] as char);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = data[i] as u32;
            out.push(B64[(n >> 2 & 0x3F) as usize] as char);
            out.push(B64[(n << 4 & 0x3F) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = (data[i] as u32) << 8 | data[i + 1] as u32;
            out.push(B64[(n >> 10 & 0x3F) as usize] as char);
            out.push(B64[(n >> 4 & 0x3F) as usize] as char);
            out.push(B64[(n << 2 & 0x3F) as usize] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// 一帧完整视频 NALU（接收端分片重组后，喂前端解码渲染）。
///
/// `nalu` 为 base64 编码的裸 H.264 NALU 字节（含 Annex-B 起始码时照原样保留）；
/// 前端 `atob` 还原后拼进 `VideoDecoder`。`is_idr` 供前端在首帧/重连后 flush 解码器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFrameMsg {
    /// 帧序号（u16 循环空间）。
    pub seq: u16,
    /// NALU 类型（`lp::datagram::NaluKind` 的 u8 值：0/1 I、2 P、3 SPS/PPS）。
    pub kind: u8,
    /// 视频 Datagram flags（`FLAG_FIRST`/`FLAG_STALE_DROP`，保留透传）。
    pub flags: u8,
    /// 完整 NALU 字节（base64）。
    pub nalu: String,
    /// 是否 IDR 关键帧（I / I+SpsPps；前端据此 flush 解码器）。
    pub is_idr: bool,
    /// 受控端采集降级 reason 码（Some(0)=无屏幕录制权限 / Some(1)=锁屏黑帧；None=真实画面）。
    /// 经视频 datagram flags 带内上报（问题 2/5），前端据此显示「当前为测试画面」banner。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_degraded: Option<u8>,
}

/// 把 [`lp::jitter::Frame`] 归一成 [`VideoFrameMsg`]（NALU → base64）。
pub fn frame_to_msg(f: &Frame) -> VideoFrameMsg {
    VideoFrameMsg {
        seq: f.seq,
        kind: f.kind,
        flags: f.flags,
        nalu: base64_encode(&f.nalu),
        is_idr: f.is_idr(),
        capture_degraded: capture_degraded_from_flags(f.flags),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp::datagram::NaluKind;

    #[test]
    fn base64_known_vectors() {
        // RFC 4648 测试向量。
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_full_byte_alphabet() {
        // 覆盖 0..255 全字节，长度 255（余 3）/ 254（余 2）/ 253（余 1）。
        let all: Vec<u8> = (0..=255).collect();
        let enc = base64_encode(&all);
        assert_eq!(enc.len(), 256usize.div_ceil(3) * 4);
        // 首 3 字节 00 01 02 → "AAEC"。
        assert!(enc.starts_with("AAEC"), "首组 0,1,2 应为 AAEC");
        // 末字节 255 单独尾组，长度 256 = 85×3+1 → 末组 1 字节 = 2 字符 + ==。
        assert!(enc.ends_with("=="));
    }

    fn frame(nalu: Vec<u8>, kind: u8, flags: u8) -> Frame {
        Frame {
            seq: 9,
            kind,
            flags,
            nalu,
        }
    }

    #[test]
    fn frame_to_msg_base64_and_flags() {
        let f = frame(vec![0, 0, 0, 1, 0x67, 0xAB], NaluKind::IdrWithSpsPps as u8, 0);
        let m = frame_to_msg(&f);
        assert_eq!(m.seq, 9);
        assert_eq!(m.kind, NaluKind::IdrWithSpsPps as u8);
        assert!(m.is_idr, "IdrWithSpsPps 判为 IDR");
        // 前 6 字节 00 00 00 01 67 AB → "AAAAAWer"（首组 00 00 00 = "AAAA"，次组 01 67 AB = "AWer"）。
        assert_eq!(m.nalu, "AAAAAWer", "NALU [0,0,0,1,0x67,0xAB] 的 base64");
    }

    #[test]
    fn frame_to_msg_p_frame_not_idr() {
        let f = frame(vec![1, 2, 3], NaluKind::PFrame as u8, lp::datagram::FLAG_FIRST);
        let m = frame_to_msg(&f);
        assert!(!m.is_idr, "P 帧非 IDR");
        assert_eq!(m.flags, lp::datagram::FLAG_FIRST);
    }

    #[test]
    fn video_frame_msg_serializes_camel_case() {
        let f = frame(vec![1, 2, 3], NaluKind::Idr as u8, 0);
        let m = frame_to_msg(&f);
        let json = serde_json::to_value(&m).expect("serialize");
        assert_eq!(json["seq"], 9);
        assert_eq!(json["isIdr"], true, "isIdr camelCase 键");
        assert!(json["nalu"].is_string(), "nalu 序列化为字符串（base64）");
    }
}
