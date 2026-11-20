//! 会话管理器（P2 遗留 ①，矩阵 §7.5「会话建立/鉴权/断开」+「画面/键鼠/剪贴板/截图/显示器」）。
//!
//! 两个宿主（`apps/ui-server` REST / `ui/src-tauri` 命令）经 `Arc<tokio::sync::Mutex<SessionManager>>`
//! 共享一个实例，暴露 4 个语义操作：
//! - [`connect`]（FR-01 一键连接）：QUIC 拨号到 `dev.ip:dev.port` + bootstrap 隧道 + ECDH 会话升级，
//!   建活跃 [`landesk_core::OneClickSession`]，折叠 `StateChanged(Running)`/`ConnModeChanged` 进状态栏快照；
//! - [`send_control`]：把 [`ControlAction`] 经 [`expand`] 展开成 1..N 条线层帧，逐帧 `send_control`
//!   （含剪贴板 32KB 分段 + SHA-256 校验，FR-07）；`Close` 意图收尾硬关；
//! - [`close`]（FR-02.4）：优雅关闭 QUIC 连接 + 状态机落 `Closed`；
//! - [`status_view`]：由状态机 + 折叠快照出前端 [`SessionStatusView`]。
//!
//! 本模块**不**做 QUIC 握手逻辑（那是 [`Controller`] 的活，127.0.0.1 环回由
//! `landesk-core/tests/oneclick` 覆盖）；「意图 → 帧」与「状态 → 视图」纯逻辑可离线单测，
//! 无网络的 `status_view`/`close`/`send_control`（未连分支）在此单测。

use landesk_core::error::CoreError;
use landesk_core::oneclick::{Controller, OneClickSession};
use landesk_core::SessionState;
use mdns::{DeviceAd, OnlineDevice};
use quic_link::{ConnMode, QuicClient};
use thiserror::Error;

use crate::session::{expand, ControlAction, ExpandError, SessionStatusView, SessionTargetView};
use crate::status::StatusBarModel;
use crate::video::{frame_to_msg, VideoFrameMsg};
use landesk_core::events::CoreEvent;
use lp::datagram::VideoDgram;
use lp::jitter::JitterBuffer;
use tokio::sync::mpsc;

/// 会话管理器：QUIC 客户端 + 活跃会话 + 目标 + 状态机 + 折叠快照。
///
/// `Session + Send + Sync`（[`OneClickSession`] 持 `quinn::Connection` + GCM，均 `Send`），
/// 可 `Arc<tokio::sync::Mutex<…>>` 跨异步任务共享；`&mut self` 方法（connect/send_control/close）
/// 经 tokio Mutex 串行化。
pub struct SessionManager {
    /// 基础 QUIC 客户端（每连克隆拨号，基础句柄留驻保持 UDP 驱动存活）。
    client: QuicClient,
    /// 活跃会话（未连为 `None`）。
    session: Option<OneClickSession>,
    /// 当前连接目标（未连为 `None`）。
    target: Option<SessionTargetView>,
    /// 会话状态机（喂 `SessionStatusView.state_code`）。
    state: SessionState,
    /// 连接方式（喂 `conn_mode_code`；内网直连恒 `Inner`）。
    mode: ConnMode,
    /// `CoreEvent` 折叠快照（喂 `encoder` 标签 + 延迟/丢包/码率/重置度量）。
    status: StatusBarModel,
    /// 视频推流泵入端（P2② 画面）：`connect` 时后台泵任务（持 `Sender`）把重组好的
    /// 完整 NALU（[`lp::jitter::JitterBuffer`] 分片拼接）写进本 `Receiver`；宿主经
    /// [`Self::take_video_rx`] 取走 `Receiver` 逐帧 `await recv()` 推给前端。
    /// `close`/再次 `connect` 时旧 `Receiver` 被 drop，泵任务 `send` 失败即退出。
    video_rx: Option<mpsc::Receiver<VideoFrameMsg>>,
}

/// `send_control` 的宿主级错误（无活跃会话 / 展开失败 / 底层会话错）。
#[derive(Debug, Error)]
pub enum SendControlError {
    /// 尚未 `connect`（无活跃会话可发控制帧）。
    #[error("no active session (connect first)")]
    NoSession,
    /// [`ControlAction`] 展开失败（剪贴板超限等）。
    #[error("expand: {0}")]
    Expand(#[from] ExpandError),
    /// 底层会话发送/加解密错。
    #[error("{0}")]
    Core(#[from] CoreError),
}

/// 会话命令面统一出参外壳（P2 遗留 ①，双宿主响应形状归一 P1.3）。
///
/// 两个宿主（`apps/ui-server` REST / `ui/src-tauri` 命令）都出**同形**结构喂前端 `landesk.ts`：
/// - `status`：当前 [`SessionStatusView`]（快照，始终给，含失败后的 `Closed`/`idle`）；
/// - `sent`：本次发出帧数（仅 `control` 成功填；`connect`/`close` 为 `None`）；
/// - `error`：可读失败原因（成功为 `None`）。
///
/// REST 侧恒 200（前端读 `error` 区分「设备不在线 / 密码错」vs 网络错）；Tauri 侧同样
/// 出本结构（前端 `invoke` 不抛，读 `error`）——**两端语义一致**（矩阵 §7.5 / P1.3）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResp {
    /// 当前会话状态快照（始终给）。
    pub status: Option<SessionStatusView>,
    /// 本次发出帧数（仅 `send_control` 成功填；其余 `None`）。
    pub sent: Option<u32>,
    /// 失败原因（成功为 `None`；`connect` 设备不在线 / 密码错、`control` 无会话 / 展开错）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SessionResp {
    /// 成功：带状态快照（`connect`/`close`/`status`）。
    pub fn ok(status: SessionStatusView) -> Self {
        Self { status: Some(status), sent: None, error: None }
    }
    /// 成功且带发出帧数（`control`）。
    pub fn ok_sent(status: SessionStatusView, sent: u32) -> Self {
        Self { status: Some(status), sent: Some(sent), error: None }
    }
    /// 失败：带（可能为空的）状态 + 原因（`status` 给失败后的最新快照，`error` 给可读消息）。
    pub fn fail(status: Option<SessionStatusView>, error: impl AsRef<str>) -> Self {
        Self {
            status,
            sent: None,
            error: Some(error.as_ref().to_string()),
        }
    }
}

impl SessionManager {
    /// 用给定 [`QuicClient`] 构造（状态机 `Idle`、无会话、无目标、无视频泵）。
    pub fn new(client: QuicClient) -> Self {
        Self {
            client,
            session: None,
            target: None,
            state: SessionState::Idle,
            mode: ConnMode::Inner,
            status: StatusBarModel::new(),
            video_rx: None,
        }
    }

    /// 用「设备ID + 明文 IP:Port」直连（不经 mDNS 在线表解析，环回 / 已知端点用）。
    ///
    /// 构造一个 [`OnlineDevice`]（`ad.id` = `device_id`、`ip`/`port` 直给、`last_seen` = 0）
    /// 走 [`Self::connect`] 的 QUIC 拨号 + 握手，避免依赖 mDNS 广播（多播需真网卡）。
    /// 受控端广播态（`enc`/`res`）此处给保守占位，仅拨号握手真正用到 `ad.id`/`ip`/`port`。
    pub async fn connect_raw(
        &mut self,
        ip: &str,
        port: u16,
        device_id: &str,
        password: &str,
        sni: &str,
    ) -> Result<SessionStatusView, CoreError> {
        let dev = OnlineDevice {
            ad: DeviceAd {
                id: device_id.into(),
                name: device_id.into(),
                enc: String::new(),
                res: String::new(),
                ver: 1,
                relay: false,
            },
            ip: ip.into(),
            port,
            last_seen: 0,
        };
        self.connect(&dev, password, sni).await
    }

    /// 取走视频推流泵入端（P2② 画面）。
    ///
    /// `connect` 成功后后台泵任务（持 `Sender`）把重组好的完整 NALU 逐帧写进
    /// [`Self::video_rx`]；本方法把该 [`mpsc::Receiver`] **移走**（置 `None`），宿主
    /// 逐帧 `await rx.recv()` 即可。未 `connect`（无活跃会话）返回 `None`。
    /// `close` 后旧 `Sender` 随 `video_rx` 被 drop 而泵任务退出，`recv()` 返回 `None`（EOS）。
    pub fn take_video_rx(&mut self) -> Option<mpsc::Receiver<VideoFrameMsg>> {
        self.video_rx.take()
    }

    /// 非阻塞取走当前已就绪的视频帧（P2② 桌面宿主轮询用，与 [`Self::take_video_rx`] 互斥）。
    ///
    /// `ui-server`（浏览器宿主）用 [`Self::take_video_rx`] 走 SSE 流；**桌面 Tauri 宿主**用本
    /// 方法 + 前端定时 `session_video_poll` 轮询（Tauri 命令不能挂住 SSE 长连接）。每次调用
    /// 排空 mpsc 缓冲里**所有**已收到的 [`VideoFrameMsg`]（`try_recv` 无阻塞）；无会话 / 无新帧
    /// 返回空 `Vec`。与 `take_video_rx` 二选一（取走接收端后本方法恒空）。
    pub fn poll_video(&mut self) -> Vec<VideoFrameMsg> {
        let Some(rx) = self.video_rx.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            out.push(msg);
        }
        out
    }

    /// 一键连接（FR-01）：QUIC 拨号 `dev.ip:dev.port` + bootstrap 认证隧道 + ECDH 会话升级。
    ///
    /// 成功返回折叠后的 [`SessionStatusView`]（`state_code="running"`）；密码错/不可达等
    /// 落 [`CoreError`]（`Auth`/`Handshake`/`Unreachable`），会话不建立。
    pub async fn connect(
        &mut self,
        dev: &OnlineDevice,
        password: &str,
        sni: &str,
    ) -> Result<SessionStatusView, CoreError> {
        // 每连克隆一个客户端拨号（基础 `self.client` 留驻保 UDP 驱动）。
        let ctrl = Controller::new(None, self.client.clone());
        let sess = ctrl.connect(dev, password, sni).await?;
        self.session = Some(sess);
        self.target = Some(SessionTargetView {
            name: dev.ad.name.clone(),
            id: dev.ad.id.clone(),
            res: (!dev.ad.res.is_empty()).then(|| dev.ad.res.clone()),
        });
        self.state = SessionState::Running;
        self.mode = ConnMode::Inner;
        // 折叠状态事件进快照（FR-09.3）。
        self.status.apply(&CoreEvent::StateChanged(SessionState::Running));
        self.status.apply(&CoreEvent::ConnModeChanged(ConnMode::Inner));
        self.start_video_pump();
        Ok(self.status_view())
    }

    /// 启动视频推流泵（P2②）：克隆会话的可共享视频 Datagram 句柄，后台循环
    /// `recv_video_datagram` → `VideoDgram::decode_plain` → [`JitterBuffer`] 分片重组 →
    /// 每凑齐一条完整 NALU 就 [`frame_to_msg`] 写进 mpsc。用**克隆句柄**（`quinn::Connection`
    /// 共享 UDP 驱动）跑独立任务，故**不持有**控制流 `&mut self` 锁——视频接收与控制帧收发
    /// 互不阻塞。`close`/再次 `connect` 时旧 `video_tx` 被 drop，旧泵任务读到 channel 关闭即退出。
    fn start_video_pump(&mut self) {
        let Some(sess) = self.session.as_ref() else {
            return;
        };
        let rx = sess.video_rx();
        let (tx, recv) = mpsc::channel::<VideoFrameMsg>(32);
        self.video_rx = Some(recv);
        tokio::spawn(async move {
            let mut jb = JitterBuffer::new();
            loop {
                // 收一个视频 Datagram；连接关闭 → 泵退出（`close` 或会话掉线）。
                let raw = match rx.recv().await {
                    Ok(b) => b,
                    Err(_) => break,
                };
                let dgram = match VideoDgram::decode_plain(&raw) {
                    Ok(d) => d,
                    // 坏包（非视频 dgram / 魔数错）：丢弃不阻断后续帧。
                    Err(_) => continue,
                };
                // 分片重组：中间片不成帧；末片到达返回完整 NALU 帧。
                if let Some(frame) = jb.push(&dgram) {
                    let msg = frame_to_msg(&frame);
                    // 宿主未取走 / 已 drop（`close` 后旧 Receiver 被丢）→ 泵退出。
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
            }
        });
    }

    /// 发送一条 [`ControlAction`]（展开成 1..N 帧逐帧 `send_control`，FR-06/FR-07）。
    ///
    /// 未 `connect` → [`SendControlError::NoSession`]；`Close` 意图收尾 [`close`]。
    /// 返回展开并成功发出的帧数。
    pub async fn send_control(&mut self, action: &ControlAction) -> Result<u32, SendControlError> {
        let frames = expand(action)?;
        let sess = self
            .session
            .as_mut()
            .ok_or(SendControlError::NoSession)?;
        for f in &frames {
            sess.send_control(f).await?;
        }
        if matches!(action, ControlAction::Close { .. }) {
            self.close();
        }
        Ok(frames.len() as u32)
    }

    /// 优雅关闭会话（FR-02.4）：硬关 QUIC 连接 + 状态机落 `Closed` + 折叠快照。
    ///
    /// 同时丢弃视频推流泵入端（`video_rx`），让后台泵任务因 `Sender` 无接收端而退出。
    pub fn close(&mut self) {
        if let Some(sess) = self.session.take() {
            sess.close(0, b"user");
        }
        self.video_rx = None;
        if self.state != SessionState::Closed {
            self.state = SessionState::Closed;
            self.status.apply(&CoreEvent::StateChanged(SessionState::Closed));
        }
    }

    /// 当前会话状态视图（未连时 `state_code="idle"`、无目标、度量为空）。
    pub fn status_view(&self) -> SessionStatusView {
        SessionStatusView::new(&self.state, self.mode, &self.status.encoder)
            .with_target(self.target.clone())
            .with_latency_ms(self.status.latency_ms)
            .with_loss_pct(self.status.loss_pct)
            .with_bitrate_kbps(self.status.bitrate_kbps)
            .with_reset(self.status.reset_events)
    }

    /// 是否已有活跃会话。
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use landesk_core::SessionState as SS;
    use lp::keycodes::Lkc;

    /// 构造一个可离线拨号的客户端（`Endpoint::client` 绑定 UDP 127.0.0.1:0，无多播、无外网）。
    fn mgr() -> SessionManager {
        let client = QuicClient::insecure_default("127.0.0.1:0".parse().expect("addr")).expect("client");
        SessionManager::new(client)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn idle_status_view() {
        let m = mgr();
        let v = m.status_view();
        assert_eq!(v.state_code, "idle", "未连 = idle");
        assert_eq!(v.state, "未连接");
        assert!(!m.is_active());
        assert!(v.target.is_none());
        assert_eq!(v.bitrate_mbps, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_control_without_session_no_session() {
        let mut m = mgr();
        let err = m
            .send_control(&ControlAction::KeyDown {
                key: Lkc::A.as_u32(),
                mods: 0,
            })
            .await
            .expect_err("未 connect 应 NoSession");
        assert!(matches!(err, SendControlError::NoSession));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn close_marks_closed() {
        let mut m = mgr();
        m.close();
        let v = m.status_view();
        assert_eq!(v.state_code, "closed", "close 后 = closed");
        assert_eq!(v.state, "已关闭");
        assert!(!m.is_active());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn close_is_idempotent_on_status() {
        let mut m = mgr();
        m.close();
        m.close();
        // 重复 close 不 panic，状态仍 Closed（快照不重复累加重置事件）。
        assert_eq!(m.status_view().state_code, "closed");
        assert_eq!(m.status_view().reset, 0);
    }

    /// `SessionState` 全 7 态码经 manager 视图自洽（与 `session::state_code_all_seven` 交叉验证）。
    #[test]
    fn manager_state_codes_match_session() {
        use crate::session::session_state_code;
        for s in [
            SS::Idle,
            SS::Auth,
            SS::Running,
            SS::Reconnecting,
            SS::AuthFailed,
            SS::NoPath,
            SS::Closed,
        ] {
            let v = SessionStatusView::new(&s, ConnMode::Inner, "");
            assert_eq!(v.state_code, session_state_code(&s));
        }
    }

    /// 无活跃会话时 `take_video_rx` 返回 `None`（未 `connect` → 无视频泵）。
    #[tokio::test(flavor = "multi_thread")]
    async fn take_video_rx_none_without_session() {
        let mut m = mgr();
        assert!(m.take_video_rx().is_none(), "未连无视频泵");
        assert!(m.poll_video().is_empty(), "无会话时 poll_video 空");
    }

    /// 端到端：受控端 `Controlled` 起 QUIC 服务端 + 发一条单片视频 Datagram，
    /// 控制端 `connect_raw`（insecure client，trust any 自签证书）建会话并起视频泵，
    /// `take_video_rx` 取泵入端，泵把收到的 dgram 经 [`lp::jitter::JitterBuffer`] 重组后
    /// 出 `VideoFrameMsg`（NALU base64 还原一致 + IDR 判定）——打通「画面推流」最薄链路。
    #[tokio::test(flavor = "multi_thread")]
    async fn video_pump_delivers_reassembled_frame() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        use landesk_core::Controlled;
        use lp::datagram::{NaluKind, FLAG_FIRST};
        use crate::video::base64_encode;

        const DEV_ID: &str = "8f3a1b2c9d0e";
        const PASSWORD: &str = "s3cret";

        // 受控端：QUIC 服务端（自签证书，insecure client 可 trust any 直接连）。
        let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let controlled = Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
        let port = controlled.server().local_addr().expect("listen addr").port();

        // 控制端：insecure client 拨号 + 握手 + 起视频泵。
        let client = QuicClient::insecure_default("127.0.0.1:0".parse().expect("127.0.0.1:0")).expect("client");
        let mut mgr = SessionManager::new(client);

        let (mut video_rx, s_session) = tokio::join!(
            async {
                let v = mgr
                    .connect_raw("127.0.0.1", port, DEV_ID, PASSWORD, "localhost")
                    .await
                    .expect("connect_raw");
                assert_eq!(v.state_code, "running", "会话建立 = running");
                mgr.take_video_rx().expect("视频泵已起")
            },
            async { controlled.wait(PASSWORD).await.expect("controlled wait") },
        );

        // 受控端发一条单片视频 Datagram（完整 NALU，末片；含 H.264 起始码）。
        let nalu: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01, NaluKind::Idr as u8 | 0x60, 0xAA];
        let vd = VideoDgram {
            seq: 1,
            kind: NaluKind::Idr as u8,
            frag: 0x80,
            flags: FLAG_FIRST,
            nalu: nalu.clone(),
        };
        s_session.send_video(&vd.encode_plain().expect("enc video dgram")).expect("send video");

        // 控制端泵收到重组好的完整帧：seq + IDR 判定 + NALU base64 还原一致。
        let got = video_rx.recv().await.expect("frame from pump");
        assert_eq!(got.seq, 1, "帧序号透传");
        assert!(got.is_idr, "kind=Idr 判为关键帧");
        assert_eq!(got.nalu, base64_encode(&nalu), "NALU 经 base64 往返一致");
    }
}
