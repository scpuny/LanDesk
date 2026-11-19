//! 一键连接握手引导（bootstrap）密钥派生（FR-02，对齐 `docs/protocol.md` §0.2 的落地简化）。
//!
//! 协议要求控制帧全程 AES-GCM 加密（NFR-03 禁裸流），但真正的会话密钥来自
//! ECDH+HKDF（M5 范围）。M4「内网一键连接」用**共享密码引导**解决鸡生蛋问题：
//! 控制端（发现到的 [`mdns::DeviceAd::id`] + 用户输入密码）与受控端（自身 `id`
//! + 配置密码）各自**独立**派生同一 `enc_key` 与两个方向性 GCM 前缀，驱动初始
//! 握手帧的加解密。
//!
//! - 密码正确 → 两侧 `enc_key` 一致 → 握手帧可互解 → 会话建立；
//! - 密码错误 → `enc_key` 不一致 → GCM tag 校验失败 → 编排层落入 `AuthFailed`。
//!
//! ECDH 会话密钥升级（`AuthRequest`/`AuthResult` 已预留 x25519 字段）留给 M5。
//! 本模块纯逻辑、零异步依赖（ADR-A1 精神），可独立单测。

use sha2::{Digest, Sha256};

/// 引导派生材料标签（与 protocol.md §0.1 的 `b"enc"` 风格一致）。
const TAG_ENC: &[u8] = b"enc";
const TAG_PREFIX_CS: &[u8] = b"cs";
const TAG_PREFIX_SC: &[u8] = b"sc";

/// 一键连接引导密钥（控制端 / 受控端各用同一 `(device_id, password)` 独立派生）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bootstrap {
    /// 会话对称加密密钥（32B，AES-256-GCM）。
    pub enc_key: [u8; 32],
    /// C→S 方向 nonce 前缀（控制端发出 / 受控端接收）。
    pub prefix_cs: [u8; 4],
    /// S→C 方向 nonce 前缀（受控端发出 / 控制端接收）。
    pub prefix_sc: [u8; 4],
}

fn sha256_of(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// 由设备 ID（mDNS `ad.id`，hex 串）与访问密码派生引导密钥。
///
/// 两个方向前缀不同，隔离 C→S 与 S→C 的 GCM nonce 空间（同一 `enc_key` 下
/// 两条流各自单调 seq，避免 nonce 复用）。
pub fn derive(device_id: &str, password: &str) -> Bootstrap {
    let id = device_id.as_bytes();
    let pw = password.as_bytes();
    let enc_key = sha256_of(&[id, pw, TAG_ENC]);
    let prefix_cs = sha256_of(&[id, pw, TAG_PREFIX_CS]);
    let prefix_sc = sha256_of(&[id, pw, TAG_PREFIX_SC]);
    Bootstrap {
        enc_key,
        prefix_cs: prefix_cs[..4].try_into().unwrap(),
        prefix_sc: prefix_sc[..4].try_into().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = derive("8f3a1b2c9d0e", "s3cret");
        let b = derive("8f3a1b2c9d0e", "s3cret");
        assert_eq!(a, b, "同 (id, pw) 必须派生同一组材料");
    }

    #[test]
    fn wrong_password_yields_different_key() {
        let a = derive("8f3a1b2c9d0e", "s3cret");
        let b = derive("8f3a1b2c9d0e", "wrong");
        assert_ne!(a.enc_key, b.enc_key);
    }

    #[test]
    fn wrong_device_id_yields_different_key() {
        let a = derive("8f3a1b2c9d0e", "s3cret");
        let b = derive("deadbeef0001", "s3cret");
        assert_ne!(a.enc_key, b.enc_key);
    }

    #[test]
    fn directions_differ_and_nonzero() {
        let b = derive("8f3a1b2c9d0e", "s3cret");
        assert_ne!(b.prefix_cs, b.prefix_sc, "双向前缀必须不同（隔离 nonce 空间）");
        assert_ne!(b.prefix_cs, [0u8; 4]);
        assert_ne!(b.prefix_sc, [0u8; 4]);
        assert_ne!(b.enc_key, [0u8; 32]);
    }
}
