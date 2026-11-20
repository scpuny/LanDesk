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
use mdns::OnlineDevice;
use quic_link::{ConnMode, QuicClient};
use thiserror::Error;

use crate::session::{expand, ControlAction, ExpandError, SessionStatusView, SessionTargetView};
use crate::status::StatusBarModel;
use landesk_core::events::CoreEvent;

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
    /// 用给定 [`QuicClient`] 构造（状态机 `Idle`、无会话、无目标）。
    pub fn new(client: QuicClient) -> Self {
        Self {
            client,
            session: None,
            target: None,
            state: SessionState::Idle,
            mode: ConnMode::Inner,
            status: StatusBarModel::new(),
        }
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
        Ok(self.status_view())
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
    pub fn close(&mut self) {
        if let Some(sess) = self.session.take() {
            sess.close(0, b"user");
        }
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
}
