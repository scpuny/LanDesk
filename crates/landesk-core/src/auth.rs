//! 会话密钥协商（M5，FR-02，对齐 `docs/protocol.md` §0.1/§0.2）。
//!
//! M4 的共享密码 bootstrap 只作**认证隧道**（加密握手帧 + 门禁密码）。M5 在其上
//! 升级为 **ECDH + HKDF 前向保密会话密钥**：握手帧携带双方 X25519 公钥与各自 nonce，
//! 两侧用「本端私钥 + 对端公钥 + client_nonce + server_nonce」独立派生同一
//! `enc_key`（`crypto::kdf`），握手成功后会话控制帧 / 视频 Datagram 全部切到它。
//!
//! ```text
//! shared   = X25519(my_priv, peer_pub)          // 对称（crypto 单测已证）
//! master   = HKDF-SHA256(ikm=shared, salt=client_nonce‖server_nonce, info="landesk-v1")
//! enc_key  = SHA256(master ‖ "enc")
//! ```
//!
//! 本模块纯逻辑、零异步依赖，可独立单测（确定性 keypair + 固定 nonce）。

use crypto::ecdh::X25519Pub;
use crypto::{
    derive_enc_key, derive_master_key, EcdhError, EcdhKeyPair,
};
use lp::msg::{AuthRequest, AuthResult};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// 前缀派生标签（方向隔离：C→S / S→C 各自 nonce 空间，避免同 enc_key 复用）。
const TAG_PREFIX_CS: &[u8] = b"m5-cs";
const TAG_PREFIX_SC: &[u8] = b"m5-sc";

/// 控制端握手材料（随机 nonce + X25519 密钥对）。
pub struct ClientHandshake {
    pub keypair: EcdhKeyPair,
    /// 控制端 32B 随机 nonce（进 HKDF salt 前 32B）。
    pub client_nonce: [u8; 32],
}

impl ClientHandshake {
    pub fn new() -> Self {
        let mut n = [0u8; 32];
        OsRng.fill_bytes(&mut n);
        Self {
            keypair: EcdhKeyPair::generate(),
            client_nonce: n,
        }
    }

    /// 构造 C→S 的 [`AuthRequest`]（携带本端 X25519 公钥 + nonce）。
    ///
    /// - `device_id`：受控端 16B ID（mDNS `ad.id`）；
    /// - `proof`：密码证明（`crypto::derive_password(password, device_id)`，不传明文，§0.2）；
    /// - `flags`：bit0=一次性密码模式（OTP 启用时置位）。
    ///
    /// 注：`server_nonce` 字段在 C→S 方向填 0（对端 nonce 尚未产生），真值在
    /// S→C 的 [`AuthResult`] 回传；两侧 `session_keys` 统一按「client 前 server 后」拼 salt。
    pub fn auth_request(&self, device_id: [u8; 16], proof: [u8; 32], flags: u8) -> AuthRequest {
        AuthRequest {
            device_id,
            server_nonce: [0u8; 32],
            client_nonce: self.client_nonce,
            client_x25519: self.keypair.public_key().0,
            proof,
            flags,
        }
    }
}

impl Default for ClientHandshake {
    fn default() -> Self {
        Self::new()
    }
}

/// 受控端握手材料（随机 nonce + X25519 密钥对）。
pub struct ServerHandshake {
    pub keypair: EcdhKeyPair,
    /// 受控端 32B 随机 nonce（进 HKDF salt 后 32B）。
    pub server_nonce: [u8; 32],
}

impl ServerHandshake {
    pub fn new() -> Self {
        let mut n = [0u8; 32];
        OsRng.fill_bytes(&mut n);
        Self {
            keypair: EcdhKeyPair::generate(),
            server_nonce: n,
        }
    }

    /// 构造 S→C 的 [`AuthResult`]（`code`：0=成功，见 §0.2 失败码表）。
    pub fn auth_result(&self, code: u8) -> AuthResult {
        AuthResult {
            code,
            server_x25519: self.keypair.public_key().0,
            server_nonce: self.server_nonce,
        }
    }
}

impl Default for ServerHandshake {
    fn default() -> Self {
        Self::new()
    }
}

/// 升级后的会话密钥材料（ECDH + HKDF）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionKeys {
    /// 会话对称密钥（32B，AES-256-GCM）。
    pub enc_key: [u8; 32],
    /// C→S 方向 nonce 前缀。
    pub prefix_cs: [u8; 4],
    /// S→C 方向 nonce 前缀。
    pub prefix_sc: [u8; 4],
}

/// 由「本端私钥 + 对端公钥 + 双 nonce」派生会话密钥（纯函数，两侧结果一致）。
///
/// - `my_keypair`：本端密钥对（**有私钥**，控制端传 client、受控端传 server）；
/// - `peer_pub`：对端 X25519 公钥（来自 `AuthRequest.client_x25519` / `AuthResult.server_x25519`）；
/// - `client_nonce` / `server_nonce`：双方各自 nonce（`AuthRequest.client_nonce` / `AuthResult.server_nonce`）。
///
/// ECDH 对称性保证控制端 `session_keys(&client.kp, server_pub, cn, sn)` 与受控端
/// `session_keys(&server.kp, client_pub, cn, sn)` 算出同一组 [`SessionKeys`]。
pub fn session_keys(
    my_keypair: &EcdhKeyPair,
    peer_pub: &X25519Pub,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> Result<SessionKeys, EcdhError> {
    let shared = my_keypair.shared_secret(peer_pub)?;
    let master = derive_master_key(&shared, client_nonce, server_nonce);
    let enc_key = derive_enc_key(&master);

    Ok(SessionKeys {
        enc_key,
        prefix_cs: prefix_of(&enc_key, TAG_PREFIX_CS, client_nonce),
        prefix_sc: prefix_of(&enc_key, TAG_PREFIX_SC, server_nonce),
    })
}

/// 由 `(enc_key, tag, nonce)` 派生 4B 方向前缀（确定性，双端一致）。
fn prefix_of(enc_key: &[u8; 32], tag: &[u8], nonce: &[u8; 32]) -> [u8; 4] {
    let mut h = Sha256::new();
    h.update(enc_key);
    h.update(tag);
    h.update(nonce);
    h.finalize()[..4].try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_client() -> ClientHandshake {
        ClientHandshake {
            keypair: EcdhKeyPair::from_raw_secret([1u8; 32]),
            client_nonce: [0x11; 32],
        }
    }

    fn fixed_server() -> ServerHandshake {
        ServerHandshake {
            keypair: EcdhKeyPair::from_raw_secret([2u8; 32]),
            server_nonce: [0x22; 32],
        }
    }

    #[test]
    fn symmetric_both_sides_same_keys() {
        let c = fixed_client();
        let s = fixed_server();
        let client_side =
            session_keys(&c.keypair, &X25519Pub(s.keypair.public_key().0), &c.client_nonce, &s.server_nonce)
                .unwrap();
        let server_side =
            session_keys(&s.keypair, &X25519Pub(c.keypair.public_key().0), &c.client_nonce, &s.server_nonce)
                .unwrap();
        assert_eq!(client_side, server_side, "ECDH 对称性 → 两侧会话密钥一致");
        assert_ne!(client_side.enc_key, [0u8; 32]);
    }

    #[test]
    fn prefixes_differ_and_deterministic() {
        let c = fixed_client();
        let s = fixed_server();
        let k1 = session_keys(&c.keypair, &X25519Pub(s.keypair.public_key().0), &c.client_nonce, &s.server_nonce).unwrap();
        let k2 = session_keys(&s.keypair, &X25519Pub(c.keypair.public_key().0), &c.client_nonce, &s.server_nonce).unwrap();
        assert_ne!(k1.prefix_cs, k1.prefix_sc, "双向前缀必须不同");
        assert_eq!(k1, k2, "前缀随 enc_key + nonce 确定性派生");
    }

    #[test]
    fn different_nonce_yields_different_key() {
        let c = fixed_client();
        let s = fixed_server();
        let a = session_keys(&c.keypair, &X25519Pub(s.keypair.public_key().0), &c.client_nonce, &s.server_nonce).unwrap();
        let b = session_keys(&c.keypair, &X25519Pub(s.keypair.public_key().0), &c.client_nonce, &[0x99; 32]).unwrap();
        assert_ne!(a.enc_key, b.enc_key, "server_nonce 参与 salt → 换 nonce 换密钥");
    }

    #[test]
    fn different_peer_pub_yields_different_key() {
        let c = fixed_client();
        let s = fixed_server();
        let other = ServerHandshake {
            keypair: EcdhKeyPair::from_raw_secret([3u8; 32]),
            server_nonce: s.server_nonce,
        };
        let a = session_keys(&c.keypair, &X25519Pub(s.keypair.public_key().0), &c.client_nonce, &c.client_nonce).unwrap();
        let b = session_keys(&c.keypair, &X25519Pub(other.keypair.public_key().0), &c.client_nonce, &other.server_nonce).unwrap();
        assert_ne!(a.enc_key, b.enc_key, "对端公钥不同 → ECDH 共享密钥不同");
    }

    #[test]
    fn auth_request_carrying_materials() {
        let c = fixed_client();
        let ar = c.auth_request([9u8; 16], [7u8; 32], 0);
        assert_eq!(ar.client_nonce, c.client_nonce);
        assert_eq!(ar.client_x25519, c.keypair.public_key().0);
        assert_eq!(ar.proof, [7u8; 32]);
    }

    #[test]
    fn auth_result_carrying_materials() {
        let s = fixed_server();
        let res = s.auth_result(0);
        assert_eq!(res.server_nonce, s.server_nonce);
        assert_eq!(res.server_x25519, s.keypair.public_key().0);
        assert_eq!(res.code, 0);
    }
}
