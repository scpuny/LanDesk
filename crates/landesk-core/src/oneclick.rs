//! 一键连接编排（FR-01「一键连接」+ FR-05「内网直连」，对齐 `docs/architecture.md` §3 landesk-core）。
//!
//! 把已实装的 [`mdns`]（发现/广播）与 [`quic_link`]（QUIC 双通道）+ [`crate::bootstrap`]
//! （共享密码引导密钥）串成端到端闭环：
//!
//! - [`Controller`]：控制端——mdns 发现局域网设备 → 选一个拨号 → bootstrap GCM 握手 →
//!   产出 [`OneClickSession`]（可收发控制帧 + 视频 Datagram，即「键鼠可用」）；
//! - [`Controlled`]：受控端——广播自身 → 等待一条 QUIC 连接 → 校验握手 → 产出 [`OneClickSession`]。
//!
//! ## 握手（M4 落地简化，ECDH/PBKDF2 会话升级归 M5）
//! 两侧用「设备ID + 访问密码」独立派生同一 `enc_key` 与方向前缀（[`Bootstrap`]），
//! 驱动初始握手帧的 AES-GCM。密码错 → `enc_key` 错 → 对端 `AuthRequest` 的 GCM tag
//! 校验失败 → 编排层落入 [`crate::session::SessionState::AuthFailed`]。`AuthRequest` /
//! `AuthResult` 中预留的 x25519 / nonce / proof 字段在 M5 填实际 ECDH + PBKDF2 材料
//! （见各字段 `TODO(M5)` 注释）。
//!
//! 本模块可被环回测试离线验证（mdns 多播部分 `#[ignore]`，QUIC + 握手部分走 127.0.0.1）。

use std::net::SocketAddr;
use std::time::Duration;

use quinn::{RecvStream, SendStream};
use tracing::{debug, warn};

use mdns::{Advertiser, Listener, OnlineDevice};
use quic_link::{LpGcm, QuicClient, QuicConnection, QuicServer};

use crate::error::CoreError;
use crate::session::SessionState;
use lp::msg::{AuthRequest, AuthResult, ControlMsg};
use lp::wire::ControlFrame;

/// 环回/测试用的握手时间窗（真实「一键发现」通常 2~5s）。
pub const DEFAULT_DISCOVER_WINDOW: Duration = Duration::from_secs(3);

/// 由 mDNS 实例名/ID 构造 16B device_id（M4 仅占位，M5 起接真实 128-bit ID）。
fn id_to_device_bytes(id: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let b = id.as_bytes();
    let n = b.len().min(16);
    out[..n].copy_from_slice(&b[..n]);
    out
}

/// M4 占位 `AuthRequest`（ECDH/PBKDF2 字段待 M5 填）。
fn m4_auth_request(device_id: [u8; 16]) -> AuthRequest {
    AuthRequest {
        device_id,
        // TODO(M5): server_nonce = 受控端随机 32B（协议 §0.2 双向 nonce 互验）
        server_nonce: [0u8; 32],
        // TODO(M5): client_nonce = 控制端随机 32B
        client_nonce: [0u8; 32],
        // TODO(M5): client_x25519 = 控制端 X25519 公钥（会话密钥协商）
        client_x25519: [0u8; 32],
        // TODO(M5): proof = SHA256(PBKDF2(password, cn‖sn))；M4 由 bootstrap GCM 承担密码校验
        proof: [0u8; 32],
        flags: 0, // bit0=一次性密码模式（M5 OTP 启用时置位）
    }
}

/// M4 占位 `AuthResult`（成功；server 侧字段待 M5 填）。
fn m4_auth_result_ok() -> AuthResult {
    AuthResult {
        code: 0,
        // TODO(M5): server_x25519 = 受控端 X25519 公钥
        server_x25519: [0u8; 32],
        // TODO(M5): server_nonce = 受控端随机 32B
        server_nonce: [0u8; 32],
    }
}

/// 双通道会话（FR-05「键鼠可用」最小闭环）。
///
/// 持有 QUIC 连接 + 一条可靠控制流 + 双向 GCM 上下文。控制帧按方向用不同
/// nonce 前缀（C→S 用 `prefix_cs`、S→C 用 `prefix_sc`），避免同 `enc_key` 下
/// 两个方向的 GCM nonce 复用。
pub struct OneClickSession {
    conn: QuicConnection,
    csend: SendStream,
    crecv: RecvStream,
    /// 本端发出方向的控制帧 GCM。
    pub tx: LpGcm,
    /// 本端接收方向的控制帧 GCM。
    pub rx: LpGcm,
    pub enc_key: [u8; 32],
    pub remote: SocketAddr,
    /// 本端在会话中的角色（喂 UI 状态栏 / 事件）。
    pub mode: quic_link::ConnMode,
    state: SessionState,
    tx_seq: u64,
    last_rx_seq: u64,
}

impl std::fmt::Debug for OneClickSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneClickSession")
            .field("remote", &self.remote)
            .field("mode", &self.mode)
            .field("state", &self.state)
            .field("tx_seq", &self.tx_seq)
            .field("last_rx_seq", &self.last_rx_seq)
            .field("enc_key", &"<redacted>")
            .finish()
    }
}

impl OneClickSession {
    /// 由握手成功后的连接 + 方向 GCM 构造（编排层内部调用）。
    fn established(
        conn: QuicConnection,
        csend: SendStream,
        crecv: RecvStream,
        enc_key: [u8; 32],
        tx: LpGcm,
        rx: LpGcm,
        mode: quic_link::ConnMode,
    ) -> Self {
        let remote = conn.remote_addr();
        Self {
            conn,
            csend,
            crecv,
            remote,
            enc_key,
            tx,
            rx,
            mode,
            state: SessionState::Running,
            // 握手帧已占用 seq=1（C→S 与 S→C 各一帧），session 控制帧从 seq=2 起，
            // 避免同 (enc_key, prefix) 下 GCM nonce 复用。
            tx_seq: 1,
            last_rx_seq: 1,
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.remote
    }

    /// 发送一条**明文语义**控制帧（自动按本端发送方向 GCM 加密 + 单调 seq）。
    pub async fn send_control(&mut self, msg: &ControlMsg) -> Result<(), CoreError> {
        self.tx_seq += 1;
        let frame = ControlFrame {
            msg: msg.clone(),
            seq: self.tx_seq,
        };
        let cipher = ControlFrame::encrypt(&self.tx, &self.enc_key, &frame)?;
        self.conn.write_control_cipher(&mut self.csend, &cipher).await?;
        Ok(())
    }

    /// 接收并解密一条控制帧（按本端接收方向 GCM，`seq` 单调 + 重放检测）。
    pub async fn recv_control(&mut self) -> Result<ControlMsg, CoreError> {
        let cipher = self.conn.read_control_cipher(&mut self.crecv).await?;
        let seq = self.last_rx_seq + 1;
        let last = self.last_rx_seq;
        let plain = ControlFrame::decrypt(&self.rx, &self.enc_key, seq, move |s| s > last, &cipher)?;
        self.last_rx_seq = seq;
        Ok(plain.msg)
    }

    /// 发一帧视频 Datagram（明文 `lp::datagram::VideoDgram::encode_plain` 输出）。
    pub fn send_video(&self, plain: &[u8]) -> Result<(), CoreError> {
        self.conn.send_video_datagram(plain)?;
        Ok(())
    }

    /// 收一帧视频 Datagram。
    pub async fn recv_video(&self) -> Result<Vec<u8>, CoreError> {
        Ok(self.conn.recv_video_datagram().await?)
    }

    /// 优雅关闭会话。
    pub fn close(&self, code: u32, reason: &[u8]) {
        self.conn.close(code, reason);
    }
}

// ---------- 控制端 ----------

/// 控制端编排：mdns 发现 + QUIC 拨号 + 握手。
pub struct Controller {
    listener: Option<Listener>,
    client: QuicClient,
}

impl Controller {
    /// `listener` 可选：环回/单测不启动真实 mDNS 守护（多播需网卡）时传 `None`，
    /// 直接用 `connect` 连手工构造的 [`OnlineDevice`]；生产环境传 `Some` 走一键发现。
    pub fn new(listener: Option<Listener>, client: QuicClient) -> Self {
        Self { listener, client }
    }

    /// 访问底层监听器（`None` 时返回 `None`；生产用于快照在线设备 / 停止浏览）。
    pub fn listener(&mut self) -> &mut Option<Listener> {
        &mut self.listener
    }

    /// 访问底层客户端（调试）。
    pub fn client(&self) -> &QuicClient {
        &self.client
    }

    /// 在时间窗内浏览局域网设备（FR-01「一键发现」）。需带监听器构造。
    pub async fn discover(&mut self, window: Duration) -> Result<Vec<OnlineDevice>, CoreError> {
        let l = self
            .listener
            .as_mut()
            .ok_or(CoreError::Unreachable("no mdns listener".into()))?;
        Ok(l.discover_within(window).await?)
    }

    /// 连接发现到的某个设备（FR-01「一键连接」）：QUIC 拨号 + bootstrap 握手。
    ///
    /// - `dev`：要连的设备（`dev.ip:dev.port` 即 QUIC 端点）；
    /// - `password`：访问密码（与受控端配置一致）；
    /// - `sni`：TLS SNI（环回传 `"localhost"`；跨机传受控端主机名）。
    pub async fn connect(
        &self,
        dev: &OnlineDevice,
        password: &str,
        sni: &str,
    ) -> Result<OneClickSession, CoreError> {
        let addr = self.resolve(dev)?;
        let bootstrap = crate::bootstrap::derive(&dev.ad.id, password);

        let conn = self
            .client
            .connect(addr, sni, bootstrap.enc_key)
            .await
            .map_err(|e| CoreError::Quic(e))?;

        // 打开控制流：C 发 AuthRequest(prefix_cs)，读 AuthResult(prefix_sc)。
        let cs = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_cs);
        let sc = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_sc);
        let (mut csend, mut crecv) = conn.open_control_stream().await?;

        let req = m4_auth_request(id_to_device_bytes(&dev.ad.id));
        let req_frame = ControlFrame {
            msg: ControlMsg::AuthReq(req),
            seq: 1,
        };
        let req_cipher = ControlFrame::encrypt(&cs, &bootstrap.enc_key, &req_frame)?;
        conn.write_control_cipher(&mut csend, &req_cipher)
            .await?;

        // 读受控端 AuthResult。
        let res_cipher = conn.read_control_cipher(&mut crecv).await?;
        let res_frame = ControlFrame::decrypt(&sc, &bootstrap.enc_key, 1, |s| s > 0, &res_cipher)?;
        match res_frame.msg {
            ControlMsg::AuthRes(r) if r.code == 0 => {
                debug!(%sni, "一键握手成功，会话 Running");
                Ok(OneClickSession::established(
                    conn,
                    csend,
                    crecv,
                    bootstrap.enc_key,
                    cs, // C 端发出方向 = prefix_cs
                    sc, // C 端接收方向 = prefix_sc
                    quic_link::ConnMode::Inner,
                ))
            }
            ControlMsg::AuthRes(r) => {
                warn!(code = r.code, "受控端拒绝握手");
                conn.close(1, b"auth rejected");
                Err(CoreError::Auth(format!("code={}", r.code)))
            }
            other => {
                warn!(?other, "握手帧类型不符");
                conn.close(2, b"bad handshake frame");
                Err(CoreError::Handshake("unexpected auth frame".into()))
            }
        }
    }

    fn resolve(&self, dev: &OnlineDevice) -> Result<SocketAddr, CoreError> {
        let s = format!("{}:{}", dev.ip, dev.port);
        s.parse()
            .map_err(|_| CoreError::Unreachable(format!("bad device addr {}", s)))
    }
}

// ---------- 受控端 ----------

/// 受控端编排：QUIC 监听 + 握手校验（可选 mdns 广播）。
pub struct Controlled {
    server: QuicServer,
    device_id: String,
    advertiser: Option<Advertiser>,
}

impl Controlled {
    /// 在 `listen` 地址起 QUIC 服务端（自签证书覆盖 `hostnames`）。
    pub fn new(
        listen: SocketAddr,
        hostnames: &[&str],
        device_id: String,
    ) -> Result<Self, CoreError> {
        let server = QuicServer::bind(listen, hostnames)?;
        Ok(Self {
            server,
            device_id,
            advertiser: None,
        })
    }

    /// 访问 QUIC 服务端（取监听地址 / 自签证书喂控制端信任根）。
    pub fn server(&self) -> &QuicServer {
        &self.server
    }

    /// 开始广播本设备到局域网（FR-01）。`port` = QUIC 监听端口，`host` 可空串。
    pub fn advertise(
        &mut self,
        ad: &mdns::DeviceAd,
        port: u16,
        host: &str,
    ) -> Result<(), CoreError> {
        let a = Advertiser::start(ad, port, host)?;
        self.advertiser = Some(a);
        Ok(())
    }

    /// 等待一条连接并完成 bootstrap 握手（密码错 → GCM tag 失败 → [`CoreError::Auth`]）。
    pub async fn wait(&self, password: &str) -> Result<OneClickSession, CoreError> {
        let bootstrap = crate::bootstrap::derive(&self.device_id, password);

        let conn = self
            .server
            .accept(bootstrap.enc_key)
            .await
            .map_err(|e| CoreError::Quic(e))?;

        // 接受控制流：S 读 AuthRequest(prefix_cs)，写 AuthResult(prefix_sc)。
        // 密码错 → bootstrap.enc_key 不一致 → 下行这步 tag 校验直接失败。
        let cs = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_cs);
        let sc = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_sc);
        let (mut ssend, mut srecv) = conn.accept_control_stream().await?;

        let req_cipher = conn.read_control_cipher(&mut srecv).await?;
        let req_frame =
            ControlFrame::decrypt(&cs, &bootstrap.enc_key, 1, |s| s > 0, &req_cipher)
                .map_err(|e| CoreError::Auth(format!("auth request 校验失败: {e}")))?;

        match req_frame.msg {
            ControlMsg::AuthReq(_) => {
                let res_frame = ControlFrame {
                    msg: ControlMsg::AuthRes(m4_auth_result_ok()),
                    seq: 1,
                };
                let res_cipher = ControlFrame::encrypt(&sc, &bootstrap.enc_key, &res_frame)?;
                conn.write_control_cipher(&mut ssend, &res_cipher)
                    .await?;
                debug!(remote = %conn.remote_addr(), "一键握手成功，会话 Running");
                Ok(OneClickSession::established(
                    conn,
                    ssend,
                    srecv,
                    bootstrap.enc_key,
                    sc, // S 端发出方向 = prefix_sc
                    cs, // S 端接收方向 = prefix_cs
                    quic_link::ConnMode::Inner,
                ))
            }
            other => {
                warn!(?other, "握手帧类型不符");
                conn.close(2, b"bad handshake frame");
                Err(CoreError::Handshake("expected AuthRequest".into()))
            }
        }
    }

    /// 优雅下线（关闭 QUIC 端点 + 注销 mDNS）。
    pub fn shutdown(&mut self) {
        self.server.close();
        if let Some(a) = self.advertiser.take() {
            drop(a); // Drop 触发 unregister
        }
    }
}
