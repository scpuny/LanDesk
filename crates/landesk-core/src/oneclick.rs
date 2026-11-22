//! 一键连接编排（FR-01「一键连接」+ FR-05「内网直连」，对齐 `docs/architecture.md` §3 landesk-core）。
//!
//! 把已实装的 [`mdns`]（发现/广播）与 [`quic_link`]（QUIC 双通道）+ [`crate::bootstrap`]
//! （共享密码引导密钥）+ [`crate::auth`]（ECDH+HKDF 会话升级）串成端到端闭环：
//!
//! - [`Controller`]：控制端——mdns 发现局域网设备 → 选一个拨号 → bootstrap GCM 隧道握手
//!   → ECDH 前向保密会话升级 → 产出 [`OneClickSession`]（可收发控制帧 + 视频 Datagram，
//!   即「键鼠可用」）；
//! - [`Controlled`]：受控端——广播自身 → 等待一条 QUIC 连接 → 密码门禁 + 校验握手 → 产出 [`OneClickSession`]。
//!
//! ## 握手（M5 落地，对齐 `docs/protocol.md` §0.2）
//! 两侧用「设备ID + 访问密码」独立派生同一 bootstrap `enc_key` 与方向前缀
//! （[`Bootstrap`]），驱动**认证隧道**（GCM 加密握手帧 + 密码门禁）。隧道内交换
//! 双方 X25519 公钥 + nonce + PBKDF2 密码证明（[`crypto::derive_password`]，不传明文，
//! NFR-03）；两侧经 [`crate::auth::session_keys`]（ECDH+HKDF）独立派生**前向保密**
//! 会话密钥，握手成功后会话控制帧 / 视频 Datagram 全部切到它。密码错 → 两侧
//! bootstrap `enc_key` 不一致 → `AuthRequest` 的 GCM tag 校验失败（或 proof 不匹配
//! 落入 [`CoreError::Auth`]）→ 会话落入 [`crate::session::SessionState::AuthFailed`]。
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
use lp::msg::ControlMsg;
use lp::wire::ControlFrame;

use crate::auth::{session_keys, ClientHandshake, ServerHandshake};
use crypto::ecdh::X25519Pub;
use crypto::derive_password;

/// 环回/测试用的握手时间窗（真实「一键发现」通常 2~5s）。
pub const DEFAULT_DISCOVER_WINDOW: Duration = Duration::from_secs(3);

/// 由 mDNS 实例名/ID 构造 16B device_id（M5 起接真实 128-bit ID）。
fn id_to_device_bytes(id: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let b = id.as_bytes();
    let n = b.len().min(16);
    out[..n].copy_from_slice(&b[..n]);
    out
}

/// 恒定时间比较两个 32B 缓冲（密码证明校验，避免时序侧信道，NFR-03）。
fn const_eq32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
            // 会话控制帧在 ECDH 密钥上从零开始（握手帧走 bootstrap 隧道，
            // 密钥与前缀均独立，nonce 空间不复用）。首帧 seq=1。
            tx_seq: 0,
            last_rx_seq: 0,
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

    /// 克隆出一个**可独立持有**的视频 Datagram 接收句柄（`Send + Sync + 'static`），
    /// 供后台推流泵 `move` 进独立 `tokio` 任务循环读帧，不占控制流 `&mut` 锁（P2② 画面推流）。
    pub fn video_rx(&self) -> quic_link::VideoDatagramRx {
        self.conn.video_rx()
    }

    /// 克隆出一个可独立持有的视频 Datagram **发送**句柄（`Send + Sync + 'static`）。
    ///
    /// 供受控端发流泵 `move` 进独立 `tokio` 任务——只发视频、不碰本会话控制流
    /// 的 `&mut`，从而与键鼠控制帧接收循环（期四）并发运行。
    pub fn video_tx(&self) -> quic_link::VideoDatagramTx {
        self.conn.video_tx()
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

    /// 连接发现到的某个设备（FR-01「一键连接」）：QUIC 拨号 + bootstrap 隧道握手 + ECDH 会话升级。
    ///
    /// - `dev`：要连的设备（`dev.ip:dev.port` 即 QUIC 端点）；
    /// - `password`：访问密码（与受控端配置一致）；
    /// - `sni`：TLS SNI（环回传 `"localhost"`；跨机传受控端主机名）。
    ///
    /// 密码走 `crypto::derive_password` 计算 32B 证明（不传明文，§0.2），
    /// 会话密钥经 ECDH+HKDF 前向保密升级（[`session_keys`]）。
    pub async fn connect(
        &self,
        dev: &OnlineDevice,
        password: &str,
        sni: &str,
    ) -> Result<OneClickSession, CoreError> {
        let addr = self.resolve(dev)?;
        let device_id = id_to_device_bytes(&dev.ad.id);
        let proof = derive_password(password, &device_id);
        let bootstrap = crate::bootstrap::derive(&dev.ad.id, password);
        let hs = ClientHandshake::new();

        let conn = self
            .client
            .connect(addr, sni, bootstrap.enc_key)
            .await
            .map_err(CoreError::Quic)?;

        // 认证隧道（bootstrap 密钥）：C 发 AuthReq(prefix_cs)，读 AuthRes(prefix_sc)。
        let cs = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_cs);
        let sc = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_sc);
        let (mut csend, mut crecv) = conn.open_control_stream().await?;

        let req = hs.auth_request(device_id, proof, 0);
        let req_frame = ControlFrame {
            msg: ControlMsg::AuthReq(req),
            seq: 1,
        };
        let req_cipher = ControlFrame::encrypt(&cs, &bootstrap.enc_key, &req_frame)?;
        conn.write_control_cipher(&mut csend, &req_cipher)
            .await?;

        // 读受控端 AuthRes。
        let res_cipher = conn.read_control_cipher(&mut crecv).await?;
        let res_frame = ControlFrame::decrypt(&sc, &bootstrap.enc_key, 1, |s| s > 0, &res_cipher)?;
        match res_frame.msg {
            ControlMsg::AuthRes(r) if r.code == 0 => {
                // ECDH 会话升级：用本端私钥 + 对端公钥 + 双 nonce 派生前向保密会话密钥。
                let sk = session_keys(
                    &hs.keypair,
                    &X25519Pub(r.server_x25519),
                    &hs.client_nonce,
                    &r.server_nonce,
                )
                .map_err(|e| CoreError::Handshake(e.to_string()))?;
                let s_tx = LpGcm::with_prefix(sk.enc_key, sk.prefix_cs);
                let s_rx = LpGcm::with_prefix(sk.enc_key, sk.prefix_sc);
                debug!(%sni, "一键握手成功（ECDH 会话升级），Running");
                Ok(OneClickSession::established(
                    conn,
                    csend,
                    crecv,
                    sk.enc_key,
                    s_tx, // C 端发出方向 = prefix_cs
                    s_rx, // C 端接收方向 = prefix_sc
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

    /// 等待一条连接并完成握手（密码门禁 + ECDH 会话升级）。
    ///
    /// 密码错 → 两侧 bootstrap enc_key 不一致 → AuthReq 的 GCM tag 校验直接失败
    /// → [`CoreError::Auth`]；或 proof 不匹配 → `AuthRes{code=1}` 后关闭。
    pub async fn wait(&self, password: &str) -> Result<OneClickSession, CoreError> {
        let device_id = id_to_device_bytes(&self.device_id);
        let expected_proof = derive_password(password, &device_id);
        let bootstrap = crate::bootstrap::derive(&self.device_id, password);
        let hs = ServerHandshake::new();

        let conn = self
            .server
            .accept(bootstrap.enc_key)
            .await
            .map_err(CoreError::Quic)?;

        // 认证隧道（bootstrap 密钥）：S 读 AuthReq(prefix_cs)，写 AuthRes(prefix_sc)。
        // 密码错 → bootstrap.enc_key 不一致 → 下行这步 tag 校验直接失败。
        let cs = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_cs);
        let sc = LpGcm::with_prefix(bootstrap.enc_key, bootstrap.prefix_sc);
        let (mut ssend, mut srecv) = conn.accept_control_stream().await?;

        let req_cipher = conn.read_control_cipher(&mut srecv).await?;
        let req_frame =
            ControlFrame::decrypt(&cs, &bootstrap.enc_key, 1, |s| s > 0, &req_cipher)
                .map_err(|e| CoreError::Auth(format!("auth request 校验失败: {e}")))?;

        match req_frame.msg {
            ControlMsg::AuthReq(r) => {
                // 密码门禁：恒时比较 PBKDF2 证明（不传/不比明文，§0.2 / NFR-03）。
                if !const_eq32(&r.proof, &expected_proof) {
                    let res = ControlFrame {
                        msg: ControlMsg::AuthRes(hs.auth_result(1)), // code=1 密码错
                        seq: 1,
                    };
                    let res_cipher = ControlFrame::encrypt(&sc, &bootstrap.enc_key, &res)?;
                    conn.write_control_cipher(&mut ssend, &res_cipher)
                        .await?;
                    warn!(remote = %conn.remote_addr(), "密码证明不匹配，拒绝");
                    conn.close(1, b"auth failed");
                    return Err(CoreError::Auth("password invalid (code=1)".into()));
                }

                // ECDH 会话升级：本端私钥 + 对端公钥 + 双 nonce。
                let sk = session_keys(
                    &hs.keypair,
                    &X25519Pub(r.client_x25519),
                    &r.client_nonce,
                    &hs.server_nonce,
                )
                .map_err(|e| CoreError::Handshake(e.to_string()))?;
                let res = ControlFrame {
                    msg: ControlMsg::AuthRes(hs.auth_result(0)),
                    seq: 1,
                };
                let res_cipher = ControlFrame::encrypt(&sc, &bootstrap.enc_key, &res)?;
                conn.write_control_cipher(&mut ssend, &res_cipher)
                    .await?;
                let s_tx = LpGcm::with_prefix(sk.enc_key, sk.prefix_sc);
                let s_rx = LpGcm::with_prefix(sk.enc_key, sk.prefix_cs);
                debug!(remote = %conn.remote_addr(), "一键握手成功（ECDH 会话升级），Running");
                Ok(OneClickSession::established(
                    conn,
                    ssend,
                    srecv,
                    sk.enc_key,
                    s_tx, // S 端发出方向 = prefix_sc
                    s_rx, // S 端接收方向 = prefix_cs
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
