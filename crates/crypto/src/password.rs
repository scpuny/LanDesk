//! 密码派生与一次性密码（协议 §0.2，NFR-03 不存明文）。
//!
//! - 预共享密码：`PBKDF2-SHA256(100_000 次, salt = device_id ‖ "landesk")` → 32B 派生值，受控端仅存派生值；
//! - 会话握手：控制端发送以静态 salt 派生的 `derived` 值，受控端比对本地存储（device_id 双方已知），
//!   通过后进入 ECDH（protocol.md §0.2 的落地简化）；
//! - 一次性密码（OTP）：受控端生成 16B 随机值，hex 展示（32 字符）供控制端输入，校验后销毁（FR-02.2）。

use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

/// PBKDF2 迭代次数（协议 §0.2）。
pub const PBKDF2_ITER: u32 = 100_000;
/// 派生值长度（字节）。
pub const DERIVED_LEN: usize = 32;
/// 静态派生 salt 后缀（协议 §0.2）。
const SALT_SUFFIX: &[u8] = b"landesk";

/// 密码模块错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OtpError {
    #[error("otp invalid: {0}")]
    Invalid(String),
}

/// 设备静态 salt：`device_id(16B) ‖ "landesk"`。
pub fn static_salt(device_id: &[u8; 16]) -> [u8; 23] {
    let mut salt = [0u8; 23];
    salt[..16].copy_from_slice(device_id);
    salt[16..].copy_from_slice(SALT_SUFFIX);
    salt
}

/// 由静态密码派生 32B 存储值（受控端只存这个，不存明文）。
pub fn derive_password(password: &str, device_id: &[u8; 16]) -> [u8; 32] {
    let salt = static_salt(device_id);
    let mut out = [0u8; DERIVED_LEN];
    // pbkdf2 0.12：`pbkdf2_hmac` 直接填充 `out`，返回 `()`（定长缓冲不会失败）。
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITER, &mut out);
    out
}

/// 受控端存储记录（32B 派生值，不落明文）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordRecord {
    pub derived: [u8; 32],
}

impl PasswordRecord {
    pub fn from_password(password: &str, device_id: &[u8; 16]) -> Self {
        Self {
            derived: derive_password(password, device_id),
        }
    }
}

/// 握手密码校验：比对控制端派生值与本地存储（恒定时间）。
pub fn verify_password_proof(record: &PasswordRecord, control_side_derived: &[u8; 32]) -> bool {
    const_eq(&record.derived, control_side_derived)
}

/// 恒定时间比较辅助（避免时序侧信道）。
fn const_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 一次性密码：随机 16B，hex 展示（32 字符）供用户输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Otp([u8; 16]);

impl Otp {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// 人类可读形式：小写 hex（32 字符）。
    pub fn display(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(32);
        for b in &self.0 {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xF) as usize] as char);
        }
        out
    }

    /// 从 display 字符串解析（控制端输入）。
    pub fn from_display(s: &str) -> Result<Self, OtpError> {
        let s = s.trim().to_ascii_lowercase();
        if s.len() != 32 {
            return Err(OtpError::Invalid(format!("expected 32 hex chars, got {}", s.len())));
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| OtpError::Invalid("non-hex character".into()))?;
        }
        Ok(Self(bytes))
    }

    /// 匹配校验（恒定时间）。
    pub fn matches(&self, other: &Otp) -> bool {
        const_eq16(&self.0, &other.0)
    }
}

fn const_eq16(a: &[u8; 16], b: &[u8; 16]) -> bool {
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let dev = [5u8; 16];
        let d1 = derive_password("s3cret", &dev);
        let d2 = derive_password("s3cret", &dev);
        assert_eq!(d1, d2);
        assert_ne!(d1, derive_password("other", &dev));
        assert_ne!(d1, derive_password("s3cret", &[6u8; 16]));
    }

    #[test]
    fn verify_proof() {
        let dev = [7u8; 16];
        let rec = PasswordRecord::from_password("pw", &dev);
        assert!(verify_password_proof(&rec, &rec.derived));
        assert!(!verify_password_proof(&rec, &[0u8; 32]));
    }

    #[test]
    fn otp_display_roundtrip() {
        let otp = Otp::generate();
        let s = otp.display();
        assert_eq!(s.len(), 32);
        let parsed = Otp::from_display(&s).unwrap();
        assert!(parsed.matches(&otp));
    }

    #[test]
    fn otp_bad_input_rejected() {
        assert!(Otp::from_display("1234").is_err());
        assert!(Otp::from_display("zz".repeat(16).as_str()).is_err());
    }
}
