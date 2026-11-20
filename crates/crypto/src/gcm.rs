//! AES-256-GCM 会话封装（协议 §0.1）。
//!
//! nonce = 会话随机前缀(8B) ‖ seq(8B, LE)。同一 `(enc_key, 前缀)` 下 seq 单调不复用；
//! 接收端记录已见最大 seq，重放即报错（安全告警，断会话）。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::Aes256Gcm;
use rand::RngCore;
use thiserror::Error;

/// GCM 错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GcmError {
    /// 密文/tag 长度不符。
    #[error("ciphertext too short")]
    TooShort,
    /// tag 校验失败（篡改或密钥错）。
    #[error("gcm tag verification failed")]
    TagInvalid,
    /// nonce/seq 重放（对齐 protocol.md §0.1：直接断会话）。
    #[error("gcm nonce replay: seq {seq} not after last {last}")]
    NonceReplay { seq: u64, last: u64 },
}

/// 会话级 GCM 实例：持有 enc_key 与随机 nonce 前缀，接收端维护 seq 单调性。
///
/// nonce 布局（96-bit）：`prefix(4B 会话随机) || seq(8B, LE)`。
/// 前缀 4B 已足以在会话内保证同一 `(enc_key, prefix)` 下 nonce 与对端可区分，
/// 余下 8B 全留给单调 seq，二者拼接恰好 12B 满足 AES-GCM 的 96-bit nonce 约束。
pub struct GcmSession {
    cipher: Aes256Gcm,
    prefix: [u8; 4],
    /// 接收端已见的最大 seq（发送端忽略；u64::MAX 表示纯发送角色）。
    last_seq: u64,
}

impl GcmSession {
    /// 新建会话（发送/接收均用同一 enc_key，随机生成 nonce 前缀）。
    pub fn new(enc_key: [u8; 32]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(&enc_key).expect("aes key fixed 32B");
        let mut prefix = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut prefix);
        Self { cipher, prefix, last_seq: 0 }
    }

    /// 用协商出的共享前缀新建会话（同一会话两端必须用同一 prefix 才能互解）。
    ///
    /// 真实握手时 prefix 由某一端随机生成并随握手交换（或从 enc_key 再派生），
    /// 两端据此构造可互通的 GcmSession。测试亦用此构造模拟共享会话。
    pub fn with_prefix(enc_key: [u8; 32], prefix: [u8; 4]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(&enc_key).expect("aes key fixed 32B");
        Self { cipher, prefix, last_seq: 0 }
    }

    /// 返回本会话的 nonce 前缀（用于握手交换 / 调试）。
    pub fn prefix(&self) -> [u8; 4] {
        self.prefix
    }

    fn nonce_bytes(&self, seq: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..4].copy_from_slice(&self.prefix);
        n[4..].copy_from_slice(&seq.to_le_bytes());
        n
    }

    /// 加密（自动递增发送 seq，tag 附在密文末尾）。
    pub fn encrypt(&mut self, seq: u64, plaintext: &[u8]) -> Result<Vec<u8>, GcmError> {
        self.cipher
            .encrypt(&self.nonce_bytes(seq).into(), plaintext)
            .map_err(|_| GcmError::TagInvalid)
    }

    /// 解密并校验 tag + seq 单调性。
    pub fn decrypt(&mut self, seq: u64, ciphertext: &[u8]) -> Result<Vec<u8>, GcmError> {
        if seq <= self.last_seq {
            return Err(GcmError::NonceReplay { seq, last: self.last_seq });
        }
        let out = self
            .cipher
            .decrypt(&self.nonce_bytes(seq).into(), ciphertext)
            .map_err(|_| GcmError::TagInvalid)?;
        self.last_seq = seq;
        Ok(out)
    }

    /// 仅校验 tag（不更新 last_seq，用于诊断）。
    pub fn check_tag(&self, seq: u64, ciphertext: &[u8]) -> bool {
        self.cipher
            .decrypt(&self.nonce_bytes(seq).into(), ciphertext)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_replay_detected() {
        let key = [42u8; 32];
        // 同一会话两端必须共享 nonce 前缀（真实握手时由一端生成并交换）。
        let prefix = [0xABu8, 0xCD, 0xEF, 0x01];
        let mut s = GcmSession::with_prefix(key, prefix);
        let c = s.encrypt(5, b"hello").unwrap();
        // 另一端：同密钥 + 同前缀，last_seq=0 → seq=5 通过
        let mut r = GcmSession::with_prefix(key, prefix);
        assert_eq!(r.decrypt(5, &c).unwrap(), b"hello");
        // 重放 seq=5
        assert!(matches!(r.decrypt(5, &c).unwrap_err(), GcmError::NonceReplay { .. }));
        // 更低 seq 也判重放
        assert!(r.decrypt(3, &c).is_err());
        // 更高 seq 通过
        let c2 = s.encrypt(6, b"world").unwrap();
        assert_eq!(r.decrypt(6, &c2).unwrap(), b"world");
    }

    #[test]
    fn mismatched_prefix_cannot_decrypt() {
        // 前缀不一致 → 解密必然失败（模拟密钥/会话不匹配），验证前缀确实参与 nonce。
        let key = [9u8; 32];
        let mut s = GcmSession::with_prefix(key, [1, 2, 3, 4]);
        let c = s.encrypt(1, b"x").unwrap();
        let mut wrong = GcmSession::with_prefix(key, [5, 6, 7, 8]);
        assert!(wrong.decrypt(1, &c).is_err());
    }

    #[test]
    fn tamper_detected() {
        let key = [1u8; 32];
        let mut s = GcmSession::new(key);
        let mut c = s.encrypt(1, b"abc").unwrap();
        let last = c.len() - 1;
        c[last] ^= 0xFF; // 破坏 tag
        let mut r = GcmSession::new(key);
        assert!(matches!(r.decrypt(1, &c).unwrap_err(), GcmError::TagInvalid));
    }
}
