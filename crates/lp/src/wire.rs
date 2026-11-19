//! 控制帧编解码（对齐 `docs/protocol.md` §1.1）。
//!
//! 帧结构（GCM 加密整个 `ver|type|len|payload` 段，tag 置于帧尾）：
//! ```text
//! +----+------+-------+----------------+--------------+
//! |ver 1|type 1|len 2LE| payload len B | gcm_tag 12B   |
//! +----+------+-------+----------------+--------------+
//! ```
//!
//! GCM 具体实例由 `crypto` crate 提供（见 [`Gcm`] trait），本模块只依赖
//! 抽象接口，保持纯逻辑可单测（ADR-A1：协议层零平台/异步依赖）。

use aes_gcm::aead::{Aead, Nonce};

use crate::error::LpError;
use crate::msg::{ControlMsg, MsgType};
use crate::{CONTROL_MAX_PAYLOAD, PROTO_VER};

/// 帧固定头部（ver+type+len）字节数。
pub const FRAME_HEADER: usize = 4;
/// GCM tag 长度。
pub const GCM_TAG_LEN: usize = 12;

/// 会话级 GCM 抽象（nonce 构造规则见 protocol.md §0.1）。
///
/// - `nonce`（96-bit）= 会话随机前缀(4B) ‖ seq(8B, LE)；
/// - 同一 `(enc_key, 前缀)` 下 seq 单调不复用；接收端可记录已见 seq，
///   重复即告警（nonce 重放检测）。
pub trait Gcm: Aead {
    /// 由会话参数生成 96-bit nonce（aead 0.5 的 `Nonce<Self>` 即 `GenericArray<u8, NonceSize>`）。
    fn nonce(&self, seq: u64) -> Nonce<Self>;
}

/// 控制帧明文（type + payload），编解码均针对此结构。
#[derive(Debug, Clone)]
pub struct ControlFrame {
    pub msg: ControlMsg,
    /// 加密/解密时的单调序号。
    pub seq: u64,
}

impl ControlFrame {
    /// 编码明文帧：`ver | type | len(2LE) | payload`。
    pub fn encode_plain(&self) -> Result<Vec<u8>, LpError> {
        let t = self.msg.msg_type();
        let payload = self.serialize_payload()?;
        let mut buf = Vec::with_capacity(FRAME_HEADER + payload.len());
        buf.push(PROTO_VER);
        buf.push(t.as_u8());
        buf.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buf.extend_from_slice(&payload);
        Ok(buf)
    }

    fn serialize_payload(&self) -> Result<Vec<u8>, LpError> {
        match &self.msg {
            ControlMsg::Heartbeat(v) => Ok(vec![*v]),
            _ => {
                let b = bincode::serialize(&self.msg).map_err(|e| LpError::Serialize(e.to_string()))?;
                if b.len() > CONTROL_MAX_PAYLOAD as usize {
                    return Err(LpError::FrameLenMismatch {
                        declared: b.len(),
                        remaining: CONTROL_MAX_PAYLOAD as usize,
                    });
                }
                Ok(b)
            }
        }
    }

    /// 解码明文帧（校验 type 与 payload 一致性，未知 type 报 [`LpError::UnknownMsgType`]）。
    pub fn decode_plain(buf: &[u8]) -> Result<ControlFrame, LpError> {
        if buf.len() < FRAME_HEADER {
            return Err(LpError::BufferTooShort { len: buf.len(), need: FRAME_HEADER });
        }
        if buf[0] != PROTO_VER {
            return Err(LpError::BadVersion(buf[0]));
        }
        let t = MsgType::from_u8(buf[1]).ok_or(LpError::UnknownMsgType(buf[1]))?;
        let len = u16::from_le_bytes([buf[2], buf[3]]) as usize;
        if buf.len() < FRAME_HEADER + len {
            return Err(LpError::BufferTooShort {
                len: buf.len(),
                need: FRAME_HEADER + len,
            });
        }
        let payload = &buf[FRAME_HEADER..FRAME_HEADER + len];
        let msg = match t {
            MsgType::Heartbeat => ControlMsg::Heartbeat(
                payload
                    .first()
                    .copied()
                    .unwrap_or(0u8),
            ),
            _ => bincode::deserialize(payload)
                .map_err(|e| LpError::Deserialize(e.to_string()))?,
        };
        // 一致性校验：反序列化结果的 type 必须与帧头一致。
        if msg.msg_type() != t {
            return Err(LpError::UnknownMsgType(t.as_u8()));
        }
        Ok(ControlFrame { msg, seq: 0 })
    }

    /// 加密帧：`encode_plain` 后经 GCM（nonce 由 seq 生成），输出含 tag。
    ///
    /// `_enc_key` 当前由 `Gcm` 实例内部持有，保留形参仅为接口稳定性（后续里程碑可能用于 AAD 绑定）。
    pub fn encrypt<E: Gcm>(gcm: &E, _enc_key: &[u8; 32], frame: &ControlFrame) -> Result<Vec<u8>, LpError> {
        let plain = frame.encode_plain()?;
        let nonce = E::nonce(gcm, frame.seq);
        let cipher = gcm
            .encrypt(&nonce, plain.as_ref())
            .map_err(|_| LpError::GcmTagInvalid)?;
        Ok(cipher)
    }

    /// 解密帧：校验 tag 后还原 [`ControlFrame`]；`expected_seq` 用于 nonce 重放检测。
    pub fn decrypt<E: Gcm>(
        gcm: &E,
        _enc_key: &[u8; 32],
        seq: u64,
        expected_nonce_ok: impl FnOnce(u64) -> bool,
        buf: &[u8],
    ) -> Result<ControlFrame, LpError> {
        if buf.len() < GCM_TAG_LEN {
            return Err(LpError::BufferTooShort { len: buf.len(), need: GCM_TAG_LEN });
        }
        if !expected_nonce_ok(seq) {
            // nonce/seq 重放（对齐 protocol.md §0.1：检测到重复直接断会话）
            return Err(LpError::GcmTagInvalid);
        }
        let nonce = E::nonce(gcm, seq);
        let plain = gcm
            .decrypt(&nonce, buf)
            .map_err(|_| LpError::GcmTagInvalid)?;
        let mut frame = ControlFrame::decode_plain(&plain)?;
        frame.seq = seq;
        Ok(frame)
    }
}
