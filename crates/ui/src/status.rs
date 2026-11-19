//! 状态栏视图模型（FR-09.3，对齐 `docs/architecture.md` §5.2 结构化字段）。
//!
//! 控制端状态栏实时展示：延迟、编码类型、连接方式（内网/P2P/中继）、丢包率。
//! `landesk-core` 经 [`landesk_core::events::CoreEvent`] 推送事件，本模型把事件
//! **归一成一个可序列化的状态栏快照**（喂 Tauri `StatusBar.vue`，`ui/frontend`）。
//!
//! 纯逻辑（事件 → 快照）可离线单测；序列化字段对齐 `ui/src-tauri` 的命令桥。

use landesk_core::events::CoreEvent;
use landesk_core::session::SessionState;
use quic_link::ConnMode;
use serde::Serialize;

/// 连接方式的展示标签（FR-09.3）。
pub fn conn_mode_label(m: ConnMode) -> &'static str {
    match m {
        ConnMode::Inner => "内网",
        ConnMode::P2p => "P2P",
        ConnMode::Relay => "中继",
    }
}

/// 编码器类型（u8）→ 展示标签；未知值 → `其他`（容错，protocol.md §7）。
pub fn encoder_label(kind: u8) -> &'static str {
    match u8_encoder_name(kind) {
        Some(n) => n,
        None => "其他",
    }
}

/// `lp::msg::EncoderKind` 的 u8 判别值 → 标签（值须与 `EncoderKind` `#[repr(u8)]` 同步）。
fn u8_encoder_name(kind: u8) -> Option<&'static str> {
    // 判别值对齐 lp::msg::EncoderKind（Qsv=1 Nvenc=2 Amf=3 V4l2m2m=4 VideoToolbox=5 Openh264=9）。
    Some(match kind {
        1 => "QSV",
        2 => "NVENC",
        3 => "AMF",
        4 => "V4L2-M2M",
        5 => "VideoToolbox",
        9 => "OpenH264",
        _ => return None,
    })
}

/// 状态栏快照（FR-09.3，序列化喂前端）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StatusBarModel {
    /// 会话状态（`Running`/`Reconnecting`/… 的 Debug 名）。
    pub state: String,
    /// 连接方式标签（`内网`/`P2P`/`中继`；未连上为空串）。
    pub conn_mode: String,
    /// 当前编码器标签。
    pub encoder: String,
    /// 端到端延迟（ms；未测得为 `None`）。
    pub latency_ms: Option<u32>,
    /// 视频丢包率（%，FR-05.5；未统计为 `None`）。
    pub loss_pct: Option<u8>,
    /// 当前码率（kbps，FR-04.5；未上报为 `None`）。
    pub bitrate_kbps: Option<u32>,
    /// 累计编码器重置事件数（FR-04.6）。
    pub reset_events: u32,
}

impl StatusBarModel {
    /// 初始：尚未连接。
    pub fn new() -> Self {
        Self::default()
    }

    /// 应用一条核心事件（`landesk-core` 事件循环里调用，喂状态栏）。
    pub fn apply(&mut self, ev: &CoreEvent) {
        match ev {
            CoreEvent::ConnModeChanged(m) => self.conn_mode = conn_mode_label(*m).to_string(),
            CoreEvent::EncoderChanged(kind) => self.encoder = encoder_label(*kind).to_string(),
            CoreEvent::LatencyMs(ms) => self.latency_ms = Some(*ms),
            CoreEvent::LossPct(p) => self.loss_pct = Some(*p),
            CoreEvent::BitrateKbps(k) => self.bitrate_kbps = Some(*k),
            CoreEvent::EncoderReset(_code) => self.reset_events = self.reset_events.saturating_add(1),
            CoreEvent::StateChanged(s) => self.state = state_label(s).to_string(),
        }
    }
}

/// 会话状态 → 展示标签（状态栏 / 设备列表角标用）。
pub fn state_label(s: &SessionState) -> &'static str {
    match s {
        SessionState::Idle => "未连接",
        SessionState::Auth => "鉴权中",
        SessionState::Running => "运行中",
        SessionState::Reconnecting => "重连中",
        SessionState::AuthFailed => "鉴权失败",
        SessionState::NoPath => "无可用路径",
        SessionState::Closed => "已关闭",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conn_mode_labels() {
        assert_eq!(conn_mode_label(ConnMode::Inner), "内网");
        assert_eq!(conn_mode_label(ConnMode::P2p), "P2P");
        assert_eq!(conn_mode_label(ConnMode::Relay), "中继");
    }

    #[test]
    fn encoder_labels() {
        // 判别值对齐 lp::msg::EncoderKind（Nvenc=2 Openh264=9）。
        assert_eq!(encoder_label(2), "NVENC");
        assert_eq!(encoder_label(9), "OpenH264");
        assert_eq!(encoder_label(0xEE), "其他", "未知编码器码值容错");
    }

    #[test]
    fn apply_updates_snapshot() {
        let mut m = StatusBarModel::new();
        m.apply(&CoreEvent::StateChanged(SessionState::Running));
        m.apply(&CoreEvent::ConnModeChanged(ConnMode::Relay));
        m.apply(&CoreEvent::EncoderChanged(1)); // Qsv=1
        m.apply(&CoreEvent::LatencyMs(24));
        m.apply(&CoreEvent::LossPct(1));
        m.apply(&CoreEvent::BitrateKbps(4000));
        m.apply(&CoreEvent::EncoderReset(1));
        m.apply(&CoreEvent::EncoderReset(2));

        assert_eq!(m.state, "运行中");
        assert_eq!(m.conn_mode, "中继");
        assert_eq!(m.encoder, "QSV");
        assert_eq!(m.latency_ms, Some(24));
        assert_eq!(m.loss_pct, Some(1));
        assert_eq!(m.bitrate_kbps, Some(4000));
        assert_eq!(m.reset_events, 2, "重置事件累加");
    }

    #[test]
    fn snapshot_is_serializable() {
        let m = StatusBarModel {
            state: "运行中".into(),
            conn_mode: "P2P".into(),
            encoder: "NVENC".into(),
            latency_ms: Some(55),
            loss_pct: Some(0),
            bitrate_kbps: Some(8000),
            reset_events: 0,
        };
        let json = serde_json::to_value(&m).expect("serialize");
        assert_eq!(json["conn_mode"], "P2P");
        assert_eq!(json["latency_ms"], 55);
    }
}
