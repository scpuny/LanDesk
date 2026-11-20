//! `crypto` — LanDesk 密钥与鉴权原语（对齐 `docs/protocol.md` §0.1 / §0.2）。
//!
//! - X25519 ECDH 协商共享密钥；
//! - HKDF-SHA256 派生会话主密钥 `master_key` 与对称加密密钥 `enc_key`；
//! - AES-256-GCM 封装 + 会话级 nonce（`rand8 ‖ seq`）构造与重放检测；
//! - PBKDF2-SHA256（100k）密码派生值存取，**不存明文**（NFR-03）；
//! - 一次性密码（OTP）生成/校验。

pub mod ecdh;
pub mod kdf;
pub mod gcm;
pub mod password;

pub use ecdh::{EcdhError, EcdhKeyPair, SharedSecret};
pub use gcm::{GcmError, GcmSession};
pub use kdf::{derive_enc_key, derive_master_key};
pub use password::{derive_password, Otp, OtpError, PasswordRecord, verify_password_proof};
