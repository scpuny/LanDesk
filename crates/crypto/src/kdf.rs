//! 密钥派生（协议 §0.1）。
//!
//! ```text
//! master_key(32B) = HKDF-SHA256(ikm = X25519 shared,
//!                    salt = client_nonce(32) ‖ server_nonce(32),
//!                    info = b"landesk-v1")
//! enc_key(32B)    = SHA256(master_key ‖ b"enc")
//! ```

use hkdf::Hkdf;
use sha2::{Digest, Sha256};

use crate::ecdh::SharedSecret;

/// HKDF 固定 info（协议 §0.1）。
pub const KDF_INFO: &[u8] = b"landesk-v1";

/// 由 X25519 共享密钥 + 双 nonce 派生会话主密钥。
pub fn derive_master_key(shared: &SharedSecret, client_nonce: &[u8; 32], server_nonce: &[u8; 32]) -> [u8; 32] {
    // salt = client_nonce(32) ‖ server_nonce(32)
    let mut salt = [0u8; 64];
    salt[..32].copy_from_slice(client_nonce);
    salt[32..].copy_from_slice(server_nonce);

    // hkdf 0.12：new(salt, ikm) 完成 extract，expand 直接写入目标缓冲。
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared.0.as_ref());
    let mut master = [0u8; 32];
    hk.expand(KDF_INFO, &mut master)
        .expect("hkdf expand: 固定 32B 输出，不可达失败");
    master
}

/// 由主密钥派生对称加密密钥。
pub fn derive_enc_key(master_key: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(master_key);
    hasher.update(b"enc");
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kdf_deterministic() {
        let shared = SharedSecret([7u8; 32]);
        let cn = [1u8; 32];
        let sn = [2u8; 32];
        let m1 = derive_master_key(&shared, &cn, &sn);
        let m2 = derive_master_key(&shared, &cn, &sn);
        assert_eq!(m1, m2, "KDF 必须确定性");
        assert_eq!(derive_enc_key(&m1), derive_enc_key(&m2));
        // 不同 nonce 产生不同密钥
        let m3 = derive_master_key(&shared, &[3u8; 32], &sn);
        assert_ne!(m1, m3);
    }

    #[test]
    fn enc_key_differs_from_master() {
        let master = [9u8; 32];
        let enc = derive_enc_key(&master);
        assert_ne!(master, enc);
    }
}
