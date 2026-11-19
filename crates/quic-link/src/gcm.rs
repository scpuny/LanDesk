//! `crypto::GcmSession` → `lp::wire::Gcm` 适配器。
//!
//! `lp::wire::ControlFrame::{encrypt,decrypt}` 通过 `Gcm: Aead` 抽象走帧加解密，
//! 需要 nonce 按 protocol.md §0.1 的布局 `prefix(4B) ‖ seq(8B, LE)` 构造。
//! 本模块提供 [`LpGcm`]：内部持 `Aes256Gcm` 裸密码器 + 4B 会话前缀，
//! `Aead::encrypt/decrypt` 委托给 `Aes256Gcm`，`Gcm::nonce` 产出 96-bit nonce。
//!
//! 与 `crypto::GcmSession` 的互通关系：
//! - 同一 `(enc_key, prefix)` 下，[`LpGcm`] 加密的帧可被
//!   `crypto::GcmSession::with_prefix(enc_key, prefix)` 解出（nonce 布局一致），
//!   反之亦然。环路测试据此校验跨 crate 一致性。

use aes_gcm::aead::{Aead, AeadCore};
use aes_gcm::aead::generic_array::typenum::consts::{U12, U16};
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::Aes256Gcm;
use aes_gcm::KeyInit;
use rand::RngCore;

use lp::wire::Gcm;

/// 会话级 GCM 实例（`lp::wire::Gcm` 的具体实现）。
///
/// nonce（96-bit）= `prefix(4B) ‖ seq(8B, LE)`。同一 `(enc_key, prefix)` 下
/// seq 单调不复用；重放检测由接收端记录已见 seq 完成（见 `lp::wire::decrypt`
/// 的 `expected_nonce_ok` 回调）。
pub struct LpGcm {
    cipher: Aes256Gcm,
    prefix: [u8; 4],
}

impl LpGcm {
    /// 用给定前缀构造会话（两端必须共享同一 prefix 才能互解）。
    ///
    /// `enc_key` 由 ECDH + HKDF 派生（`crypto::kdf::derive_enc_key`），
    /// `prefix` 由握手交换（或由 `crypto::GcmSession::new` 生成后经控制流下发）。
    pub fn with_prefix(enc_key: [u8; 32], prefix: [u8; 4]) -> Self {
        let cipher = Aes256Gcm::new_from_slice(&enc_key).expect("aes key fixed 32B");
        Self { cipher, prefix }
    }

    /// 随机前缀便捷构造（等价 `crypto::GcmSession::new`；两端仍需交换 `prefix()` 互通）。
    pub fn new(enc_key: [u8; 32]) -> Self {
        let mut prefix = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut prefix);
        Self::with_prefix(enc_key, prefix)
    }

    /// 会话 nonce 前缀（握手交换 / 调试用）。
    pub fn prefix(&self) -> [u8; 4] {
        self.prefix
    }

    fn nonce_bytes(&self, seq: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..4].copy_from_slice(&self.prefix);
        n[4..].copy_from_slice(&seq.to_le_bytes());
        n
    }
}

// ---------- lp::wire::Gcm（其超类 Aead / AeadCore 委托给 Aes256Gcm）----------

impl AeadCore for LpGcm {
    type NonceSize = U12; // 96-bit nonce = 4B prefix ‖ 8B seq
    type TagSize = U16; // GCM 16B tag
    type CiphertextOverhead = U16;
}

impl Aead for LpGcm {
    fn encrypt<'msg, 'aad>(
        &self,
        nonce: &GenericArray<u8, U12>,
        plaintext: impl Into<aes_gcm::aead::Payload<'msg, 'aad>>,
    ) -> Result<Vec<u8>, aes_gcm::aead::Error> {
        // Aes256Gcm 的 NonceSize 亦为 U12，nonce 类型可直接复用。
        self.cipher.encrypt(nonce, plaintext)
    }

    fn decrypt<'msg, 'aad>(
        &self,
        nonce: &GenericArray<u8, U12>,
        ciphertext: impl Into<aes_gcm::aead::Payload<'msg, 'aad>>,
    ) -> Result<Vec<u8>, aes_gcm::aead::Error> {
        self.cipher.decrypt(nonce, ciphertext)
    }
}

impl Gcm for LpGcm {
    fn nonce(&self, seq: u64) -> GenericArray<u8, U12> {
        GenericArray::from(self.nonce_bytes(seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp::msg::ControlMsg;
    use lp::wire::ControlFrame;

    #[test]
    fn control_frame_roundtrip_over_gcm() {
        let enc_key = [7u8; 32];
        let prefix = [0x11, 0x22, 0x33, 0x44];
        let gcm = LpGcm::with_prefix(enc_key, prefix);

        let frame = ControlFrame {
            msg: ControlMsg::Heartbeat(0x07),
            seq: 3,
        };
        let cipher = ControlFrame::encrypt(&gcm, &enc_key, &frame).expect("encrypt");
        assert!(cipher.len() >= 4 + 1 + 12, "cipher + tag length");

        // 同密钥 + 同前缀可解；expected_nonce_ok 放行 seq=3
        let r = LpGcm::with_prefix(enc_key, prefix);
        let plain =
            ControlFrame::decrypt(&r, &enc_key, 3, |_| true, &cipher).expect("decrypt");
        assert!(matches!(plain.msg, ControlMsg::Heartbeat(0x07)));
        assert_eq!(plain.seq, 3);

        // 错前缀解不开（验证 prefix 参与 nonce）
        let wrong = LpGcm::with_prefix(enc_key, [0x99, 0, 0, 0]);
        assert!(ControlFrame::decrypt(&wrong, &enc_key, 3, |_| true, &cipher).is_err());
    }

    #[test]
    fn interop_with_crypto_gcm_session() {
        // LpGcm 加密 → crypto::GcmSession 解密（同一 enc_key + prefix），
        // 证明跨 crate nonce 布局一致（4B ‖ 8B）。
        use crypto::GcmSession;
        let enc_key = [42u8; 32];
        let prefix = [0xAB, 0xCD, 0xEF, 0x01];

        let gcm = LpGcm::with_prefix(enc_key, prefix);
        let frame = ControlFrame {
            msg: ControlMsg::Heartbeat(0x10),
            seq: 5,
        };
        let cipher = ControlFrame::encrypt(&gcm, &enc_key, &frame).unwrap();

        // crypto 侧同一 (enc_key, prefix)，last_seq=0 → seq=5 通过
        let mut session = GcmSession::with_prefix(enc_key, prefix);
        let plain = session
            .decrypt(5, &cipher)
            .expect("LpGcm output must be GcmSession-decryptable");
        assert_eq!(plain.len(), 4 + 1, "ver|type|len2|payload(1B heartbeat)");
    }
}
