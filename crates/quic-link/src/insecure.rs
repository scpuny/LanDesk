//! 信任任意证书的 QUIC 客户端验证器（FR-01「双机内网一键连接」，对齐 `architecture.md` §3）。
//!
//! 内网跨机场景控制端**无法预置对端自签证书**（不像环回测试能把同机证书塞进信任根），
//! 故走 trust-any：跳过证书链 + hostname 校验（LAN 内网信任域假设），但**保留 TLS1.3
//! Finished 消息的签名验证**（防 MITM 篡改会话密钥与握手状态）——安全边界由
//! `landesk-core` 的 bootstrap GCM 密码校验兜底（RustDesk-LAN 同款模型，protocol.md §0）。
//!
//! 仅在 LAN 内网直连使用。外网 P2P / 中继（M6/M7）必须替换为正式 CA 或证书交换。

use std::sync::Arc;

use quinn::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use quinn::rustls::crypto::{verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use quinn::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use quinn::rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme, version};

/// 信任任意服务器证书的验证器（保留 TLS1.3 签名校验）。
///
/// 持有一组签名方案（来自 provider 的 `signature_verification_algorithms`），用于：
/// - 向客户端通告本端支持的签名方案（`supported_verify_schemes`）；
/// - 验证 TLS1.3 `Finished` 消息签名（`verify_tls13_signature`）。
#[derive(Debug)]
pub struct TrustAny {
    schemes: WebPkiSupportedAlgorithms,
}

impl TrustAny {
    /// 用给定 provider 的签名验证算法集构造。
    pub fn new(provider: &CryptoProvider) -> Self {
        Self {
            schemes: provider.signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for TrustAny {
    /// 信任任意证书（LAN 内网信任域假设；外网 MUST NOT 使用本验证器）。
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    /// QUIC 仅用 TLS1.3，此方法实际不会被调用。
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TrustAny: TLS1.2 unsupported".into()))
    }

    /// 保留 TLS1.3 `Finished` 签名校验（防 MITM 篡改会话密钥）。
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.schemes)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.schemes.supported_schemes()
    }
}

/// 构建一个「信任任意证书」的 quinn `ClientConfig`（保留 TLS1.3 签名校验）。
///
/// 用于 LAN 跨机一键连接；控制端拿不到受控端自签证书（环回测试才用
/// `QuicClient::new` + 已知信任根）。外网场景禁用。
pub fn insecure_client_config(provider: &CryptoProvider) -> Result<quinn::ClientConfig, Error> {
    let verifier: Arc<dyn ServerCertVerifier> = Arc::new(TrustAny::new(provider));
    let crypto = ClientConfig::builder_with_provider(Arc::new(provider.clone()))
        .with_protocol_versions(&[&version::TLS13])
        .map_err(|_| Error::General("no usable TLS1.3 suites in provider".into()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(Arc::new(crypto))
            .map_err(|_| Error::General("no initial cipher suite".into()))?,
    )))
}
