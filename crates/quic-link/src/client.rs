//! QUIC 客户端（FR-09.3 控制端）。
//!
//! 持 [`quinn::Endpoint`] + 信任根（默认信任服务端自签证书）。[`connect`]
//! 拨号到 `server_addr` 并返回双通道句柄 [`QuicConnection`]。

use std::net::SocketAddr;
use std::sync::Arc;

use quinn::rustls::crypto::CryptoProvider;
use quinn::rustls::RootCertStore;
use quinn::{ClientConfig, Endpoint, TransportConfig};

use crate::conn::QuicConnection;
use crate::error::QuicLinkError;
use crate::gcm::LpGcm;
use crate::insecure;
use crate::tls::SelfSigned;

/// QUIC 客户端。
///
/// 可 [`Clone`]：底层 [`quinn::Endpoint`] 共享同一 UDP 驱动，克隆句柄拨号同一端点，
/// 宿主（会话管理器）每连克隆一个、同时保留基础句柄让驱动存活。
#[derive(Debug, Clone)]
pub struct QuicClient {
    endpoint: Endpoint,
}

impl QuicClient {
    /// 创建客户端并信任给定自签证书（`QuicServer::tls()` 同源）。
    ///
    /// `bind` 地址通常为 `0.0.0.0:0` / `[::]:0`（本地源端口由系统分配）。
    pub fn new(
        bind: SocketAddr,
        tls: &SelfSigned,
    ) -> Result<Self, QuicLinkError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(tls.cert_der.clone())
            .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;

        let mut client_config = ClientConfig::with_root_certificates(Arc::new(roots))
            .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        client_config.transport_config(Arc::new(TransportConfig::default()));

        let mut endpoint =
            Endpoint::client(bind).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// 创建「信任任意证书」客户端（LAN 跨机一键连接，FR-01）。
    ///
    /// 跳过证书链/hostname 校验（内网信任域假设），保留 TLS1.3 握手签名校验；
    /// 端到端机密性由会话 GCM（bootstrap 密码）兜底。**外网场景禁用**——
    /// 须换正式 CA 或证书交换（M6/M7）。`provider` 为 quinn 内置 ring provider。
    pub fn insecure(
        bind: SocketAddr,
        provider: &CryptoProvider,
    ) -> Result<Self, QuicLinkError> {
        let client_config =
            insecure::insecure_client_config(provider)
                .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        let mut client_config = client_config;
        client_config.transport_config(Arc::new(TransportConfig::default()));

        let mut endpoint =
            Endpoint::client(bind).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// 创建「信任任意证书」客户端，**自动装 quinn 内置 ring provider**（宿主便捷入口）。
    ///
    /// 等价 [`Self::insecure`] 但内部用 `quinn::rustls::crypto::ring::default_provider()`
    /// （需 quinn 编译进 ring 后端，本仓默认即如此）——宿主（`landesk-ui` 命令面 /
    /// `apps/ui-server`）**不必直接依赖 `quinn`/`ring`**，经 `landesk-ui` re-export 调本方法即可。
    /// 一次性 `install_default()`（幂等，已装则忽略重复安装）。
    pub fn insecure_default(bind: SocketAddr) -> Result<Self, QuicLinkError> {
        use quinn::rustls::crypto::ring::default_provider;
        let provider = default_provider();
        let _ = provider.clone().install_default();
        Self::insecure(bind, &provider)
    }

    /// 连接 `server_addr`，`server_name` 为 TLS SNI（环路测试传 `"localhost"`）。
    ///
    /// 会话 GCM 用给定 `enc_key` + 随机前缀构造；两端需共享同一 `enc_key`
    /// 并交换前缀后互通（M3 环路测试两端同 key，前缀经控制流下发）。
    pub async fn connect(
        &self,
        server_addr: SocketAddr,
        server_name: &str,
        enc_key: [u8; 32],
    ) -> Result<QuicConnection, QuicLinkError> {
        let connecting = self
            .endpoint
            .connect(server_addr, server_name)
            .map_err(|e| QuicLinkError::ConnectFailed(e.to_string()))?;
        let conn = connecting
            .await
            .map_err(|e| QuicLinkError::ConnectFailed(e.to_string()))?;

        let gcm = LpGcm::with_prefix(enc_key, [0u8; 4]);
        Ok(QuicConnection::new(conn, gcm))
    }

    /// 本端本地地址（调试 / 环路日志）。
    pub fn local_addr(&self) -> Result<SocketAddr, QuicLinkError> {
        self.endpoint
            .local_addr()
            .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))
    }

    /// 优雅关闭本端点。
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"client close");
    }

    /// 直接暴露 endpoint（高级用法，如自定义 transport config）。
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}
