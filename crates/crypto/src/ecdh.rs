//! X25519 ECDH（协议 §0.1）。

use rand::rngs::OsRng;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

/// X25519 公钥（32B，可序列化进 AuthRequest/AuthResult）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X25519Pub(pub [u8; 32]);

impl X25519Pub {
    fn to_dalek(self) -> PublicKey {
        PublicKey::from(self.0)
    }
}

/// ECDH 密钥对（私钥不落盘，会话级）。
///
/// 使用 `StaticSecret` 持有可复用的私钥（x25519-dalek 2.0 起旧 `X25519` 类型已移除）。
pub struct EcdhKeyPair {
    priv_key: StaticSecret,
    pub_key: X25519Pub,
}

impl std::fmt::Debug for EcdhKeyPair {
    // 私钥是敏感材料，Debug 只展示公钥，避免日志泄漏。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcdhKeyPair")
            .field("pub_key", &self.pub_key)
            .field("priv_key", &"<redacted>")
            .finish()
    }
}

/// ECDH 共享密钥（32B，作为 HKDF 的 IKM，见 kdf 模块）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSecret(pub [u8; 32]);

/// ECDH 错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EcdhError {
    #[error("invalid public key length: {0}")]
    BadPubKeyLen(usize),
}

impl EcdhKeyPair {
    /// 生成随机密钥对。
    pub fn generate() -> Self {
        let priv_key = StaticSecret::random_from_rng(OsRng);
        let pub_key = X25519Pub(PublicKey::from(&priv_key).to_bytes());
        Self { priv_key, pub_key }
    }

    /// 由 32B 原始私钥恢复（调试/测试用；生产不建议长期保存私钥）。
    pub fn from_raw_secret(secret: [u8; 32]) -> Self {
        let priv_key = StaticSecret::from(secret);
        let pub_key = X25519Pub(PublicKey::from(&priv_key).to_bytes());
        Self { priv_key, pub_key }
    }

    pub fn public_key(&self) -> &X25519Pub {
        &self.pub_key
    }

    /// 与对方公钥计算共享密钥。
    pub fn shared_secret(&self, peer: &X25519Pub) -> Result<SharedSecret, EcdhError> {
        let shared = self.priv_key.diffie_hellman(&peer.to_dalek());
        Ok(SharedSecret(shared.to_bytes()))
    }
}

/// 校验 32B 公钥长度（X25519 公钥无格式校验，长度即校验）。
pub fn validate_pub_key(raw: &[u8]) -> Result<[u8; 32], EcdhError> {
    if raw.len() != 32 {
        return Err(EcdhError::BadPubKeyLen(raw.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(raw);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecdh_shared_secret_is_symmetric() {
        let alice = EcdhKeyPair::generate();
        let bob = EcdhKeyPair::generate();
        let s_ab = alice.shared_secret(bob.public_key()).unwrap();
        let s_ba = bob.shared_secret(alice.public_key()).unwrap();
        assert_eq!(s_ab.0, s_ba.0, "ECDH 共享密钥必须对称");
    }

    #[test]
    fn invalid_pub_len_rejected() {
        let kp = EcdhKeyPair::generate();
        assert!(validate_pub_key(&[0u8; 31]).is_err());
        let good: [u8; 32] = kp.public_key().0;
        assert!(validate_pub_key(&good).is_ok());
    }
}
