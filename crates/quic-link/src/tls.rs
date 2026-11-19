//! 自签证书（M3 环路测试 / 内网直连 TLS 用）。
//!
//! QUIC 强制 TLS 1.3。内网直连与环路测试无法预置受信 CA，故用 [`rcgen`]
//! 生成单证书自签：服务端持证书 + 私钥，客户端把同一自签证书加入
//! `RootCertStore` 信任根即可互通（`docs/protocol.md` §0：会话内 ECDH
//! 派生密钥独立于 TLS——TLS 仅负责通道完整性与防中间人起点，端到端
//! 机密性仍由会话 GCM 承担）。
//!
//! 外网 P2P / 中继（M5/M6）将替换为正式 CA 签发或证书交换机制。

use rcgen::{Certificate, CertificateParams, KeyPair};
use quinn::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use thiserror::Error;

/// 证书 / TLS 配置错误。
#[derive(Debug, Error)]
pub enum TlsError {
    #[error("rcgen cert generation failed: {0}")]
    Rcgen(String),
}

/// 自签证书对（服务端持用）。
///
/// 注意：`key`（[`PrivateKeyDer`]）本身不可 `Clone`，需要副本时调
/// `key.clone_key()`；`cert_der` 可自由 `clone()`。
pub struct SelfSigned {
    /// 证书 DER（客户端信任根 + 服务端 `with_single_cert` 共用）。
    pub cert_der: CertificateDer<'static>,
    /// 服务端私钥（`with_single_cert` 用，PKCS#8）。
    pub key: PrivateKeyDer<'static>,
}

/// 生成一张覆盖给定主机名 / IP 的自签证书。
///
/// `hostnames` 里能解析为 IP 的自动进 `IpAddress` SAN，其余进 `DnsName`
/// SAN（由 [`CertificateParams::new`] 处理）。默认额外覆盖 127.0.0.1 / ::1，
/// 环路测试稳过。
pub fn self_signed(hostnames: &[&str]) -> Result<SelfSigned, TlsError> {
    let key = KeyPair::generate().map_err(|e| TlsError::Rcgen(e.to_string()))?;

    let mut sans: Vec<String> = hostnames.iter().map(|s| s.to_string()).collect();
    sans.push("127.0.0.1".into());
    sans.push("::1".into());

    let params =
        CertificateParams::new(sans).map_err(|e| TlsError::Rcgen(e.to_string()))?;
    let cert: Certificate = params
        .self_signed(&key)
        .map_err(|e| TlsError::Rcgen(e.to_string()))?;

    let cert_der: CertificateDer<'static> = cert.der().clone();
    let key_der = key.serialize_der();
    let key: PrivateKeyDer<'static> = PrivateKeyDer::Pkcs8(key_der.into());

    Ok(SelfSigned { cert_der, key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_parseable_self_signed_cert() {
        let ss = self_signed(&["localhost"]).expect("self_signed");
        // 证书 DER 以 PKCS#7/CERTIFICATE 的 SEQUENCE(0x30) 开头；私钥非空。
        assert!(ss.cert_der.as_ref().starts_with(&[0x30]));
        assert!(!ss.key.secret_der().is_empty());
    }
}
