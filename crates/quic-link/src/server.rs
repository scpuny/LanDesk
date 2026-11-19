//! QUIC 服务端（FR-05 / FR-09.3 内网直连受控端）。
//!
//! 持 [`quinn::Endpoint`] + 自签证书 + 会话 GCM。[`accept`] 阻塞等待一条
//! 新连接，返回 [`QuicConnection`]（双通道句柄）。

use std::net::SocketAddr;
use std::sync::Arc;

use quinn::{Endpoint, ServerConfig, TransportConfig};

use crate::conn::QuicConnection;
use crate::error::QuicLinkError;
use crate::gcm::LpGcm;
use crate::tls::{self, SelfSigned};

/// QUIC 服务端。
///
/// 一个 [`QuicServer`] 可同时服务多条连接（每条 `accept` 返回一个独立
/// [`QuicConnection`]）。
pub struct QuicServer {
    endpoint: Endpoint,
    /// 自签证书（客户端信任根来源）。
    tls: SelfSigned,
}

impl QuicServer {
    /// 在 `listen` 地址监听（`port=0` 时由系统分配，见 [`local_addr`]）。
    ///
    /// 用给定主机名生成自签证书；`enc_key` 为会话对称密钥（真实握手时来自
    /// ECDH+HKDF，M3 环路测试两端共享固定值即可）。
    pub fn bind(
        listen: SocketAddr,
        hostnames: &[&str],
    ) -> Result<Self, QuicLinkError> {
        let tls = tls::self_signed(hostnames).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        Self::bind_with_tls(listen, tls)
    }

    /// 用已有自签证书对监听（便于测试共享同一证书到客户端信任根）。
    pub fn bind_with_tls(listen: SocketAddr, tls: SelfSigned) -> Result<Self, QuicLinkError> {
        let mut server_config =
            ServerConfig::with_single_cert(vec![tls.cert_der.clone()], tls.key.clone_key())
                .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        server_config.transport_config(Arc::new(TransportConfig::default()));

        let endpoint =
            Endpoint::server(server_config, listen).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;

        Ok(Self { endpoint, tls })
    }

    /// 本地监听地址（`port=0` 时取实际分配端口）。
    pub fn local_addr(&self) -> Result<SocketAddr, QuicLinkError> {
        self.endpoint
            .local_addr()
            .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))
    }

    /// 自签证书（客户端 `QuicClient::new` 信任它）。
    pub fn tls(&self) -> &SelfSigned {
        &self.tls
    }

    /// 等待一条新连接，返回双通道句柄。
    ///
    /// `enc_key` 为会话对称密钥，与客户端 `connect(enc_key)` 共享同一值即可互通。
    pub async fn accept(&self, enc_key: [u8; 32]) -> Result<QuicConnection, QuicLinkError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(QuicLinkError::AcceptFailed("endpoint closed".into()))?;
        let connecting = incoming.accept().map_err(|e| QuicLinkError::AcceptFailed(e.to_string()))?;
        let conn = connecting
            .await
            .map_err(|e| QuicLinkError::ConnectFailed(e.to_string()))?;

        let gcm = LpGcm::with_prefix(enc_key, [0u8; 4]);
        Ok(QuicConnection::new(conn, gcm))
    }

    /// 优雅关闭本端点（不再接受新连接，已有连接自然断开）。
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"server close");
    }
}
