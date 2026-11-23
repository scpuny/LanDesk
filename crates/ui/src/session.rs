//! 会话命令面（P2 遗留 ①，矩阵 §7.5「会话建立/鉴权/断开」+「画面/键鼠/剪贴板/截图/显示器」）。
//!
//! 把 UI 层的**控制意图**（[`ControlAction`]）归一成 [`lp`] 线层控制帧（[`ControlMsg`]），
//! 供宿主（`apps/ui-server` REST / `ui/src-tauri` 命令）经 [`landesk_core::OneClickSession`]
//! 的 `send_control` 发出去——即 `session_send_control` 命令的可离线验证内核。
//!
//! 本模块**纯逻辑**（不依赖 `tracing`/`quinn`，只 `landesk-core`/`lp`/`sha2`），
//! 可离线单测；真正的会话建立/断开（QUIC + 握手）由宿主调
//! [`landesk_core::Controller`] / [`landesk_core::Controlled`] 完成，本模块只管
//! 「意图 → 帧」与「状态 → 视图」。剪贴板走 [`lp::CLIP_CHUNK_CAP`] 定长分段 + SHA-256 校验（FR-07）。

use sha2::{Digest, Sha256};

use landesk_core::SessionState;
use lp::msg::{
    CLIP_CHUNK_CAP, CLIP_TOTAL_CAP, ControlMsg, DisplaySwitch, EncodeParams, KeyEvent,
    MouseButton, MouseMove, MouseWheel, Ping,
};
use quic_link::ConnMode;
use serde::{Deserialize, Serialize};

// 键盘修饰键位（与 `lp::keycodes` `MOD_*` 对齐，FR-06.2）。
pub const MOD_CTRL: u8 = lp::keycodes::MOD_CTRL;
pub const MOD_SHIFT: u8 = lp::keycodes::MOD_SHIFT;
pub const MOD_ALT: u8 = lp::keycodes::MOD_ALT;
pub const MOD_META: u8 = lp::keycodes::MOD_META;

/// UI 层控制意图（前端工具栏/快捷键的抽象，`camelCase` 序列化对齐前端）。
///
/// 由 [`expand`] 展开成 1..N 条 [`ControlMsg`]：键鼠/编码器/显示器/会话各 1 帧，
/// 剪贴板文本展开为 `ClipStart + ClipChunk* + ClipEnd`（末段可短，FR-07）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlAction {
    /// 鼠标移动（坐标归一化 0..=65535，FR-06.3）。
    MouseMove { x: u16, y: u16, buttons: u16, mods: u8 },
    /// 鼠标按键（`button`: 0 左 1 右 2 中；`down`: 1 按下 0 抬起）。
    MouseButton { button: u8, down: u8 },
    /// 滚轮（负 = 下/左）。
    MouseWheel { dx: i16, dy: i16 },
    /// 键盘按下（`key` 为 LKC，见 [`Lkc`]）。
    KeyDown { key: u32, mods: u8 },
    /// 键盘抬起。
    KeyUp { key: u32, mods: u8 },
    /// 剪贴板同步（MVP 仅文本，FR-07）：展开为分段 + SHA-256 校验。
    ClipboardText { text: String },
    /// 运行时码率 / I 帧间隔 / 预设调整（FR-04.5）。
    EncodeParams { bitrate_kbps: u16, keyframe_interval: u16, preset: u8 },
    /// 显示器切换（FR-03.3）。
    DisplaySwitch { index: u8 },
    /// 心跳（会话保活）。
    Heartbeat { code: u8 },
    /// 延迟探测（PING/PONG 测得，FR-09.3）。
    Ping { ts: u64 },
    /// 关闭会话（`reason` 对齐 [`lp::msg::CloseReason`] 的 u8）。
    Close { reason: u8 },
}

/// 单帧展开错误（剪贴板超限 / 未知键码）。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExpandError {
    /// 剪贴板文本超过 [`lp::CLIP_TOTAL_CAP`]（1MB，FR-07.2）。
    #[error("clipboard too large: {0} bytes (cap {1})")]
    ClipTooLarge(usize, u32),
}

/// 展开一个 [`ControlAction`] 为 1..N 条线层 [`ControlMsg`]（宿主逐帧 `send_control`）。
pub fn expand(action: &ControlAction) -> Result<Vec<ControlMsg>, ExpandError> {
    Ok(match action {
        ControlAction::MouseMove { x, y, buttons, mods } => {
            vec![ControlMsg::MouseMove(MouseMove {
                x: *x,
                y: *y,
                buttons: *buttons,
                mods: *mods,
            })]
        }
        ControlAction::MouseButton { button, down } => vec![ControlMsg::MouseButton(
            MouseButton {
                button: *button,
                down: *down,
            },
        )],
        ControlAction::MouseWheel { dx, dy } => {
            vec![ControlMsg::MouseWheel(MouseWheel {
                dx: *dx,
                dy: *dy,
            })]
        }
        ControlAction::KeyDown { key, mods } => vec![ControlMsg::KeyDown(KeyEvent {
            key: *key,
            mods: *mods,
        })],
        ControlAction::KeyUp { key, mods } => vec![ControlMsg::KeyUp(KeyEvent {
            key: *key,
            mods: *mods,
        })],
        ControlAction::ClipboardText { text } => {
            clip_text_to_msgs(text.as_bytes())?
        }
        ControlAction::EncodeParams {
            bitrate_kbps,
            keyframe_interval,
            preset,
        } => vec![ControlMsg::EncodeParams(EncodeParams {
            bitrate_kbps: *bitrate_kbps,
            keyframe_interval: *keyframe_interval,
            preset: *preset,
        })],
        ControlAction::DisplaySwitch { index } => {
            vec![ControlMsg::DisplaySwitch(DisplaySwitch {
                index: *index,
            })]
        }
        ControlAction::Heartbeat { code } => vec![ControlMsg::Heartbeat(*code)],
        ControlAction::Ping { ts } => vec![ControlMsg::Ping(Ping { ts: *ts })],
        ControlAction::Close { reason } => {
            vec![ControlMsg::Close(lp::msg::SessionClose {
                reason: *reason,
            })]
        }
    })
}

/// 把文本剪贴板展开为 `ClipStart` + 定长 `ClipChunk*`（32KB，末段可短）+ `ClipEnd(sha256)`。
///
/// `total_len` = 字节总数；分段边界按 [`CLIP_CHUNK_CAP`]；SHA-256 覆盖整段文本（FR-07.2 校验）。
pub fn clip_text_to_msgs(data: &[u8]) -> Result<Vec<ControlMsg>, ExpandError> {
    let total = data.len() as u32;
    if total > CLIP_TOTAL_CAP {
        return Err(ExpandError::ClipTooLarge(data.len(), CLIP_TOTAL_CAP));
    }
    let mut out: Vec<ControlMsg> = Vec::new();
    out.push(ControlMsg::ClipStart(lp::msg::ClipboardStart {
        total_len: total,
        mime_flags: 0, // MVP 仅文本（mime_flags 预留）
    }));
    let mut seq: u16 = 0;
    let mut off = 0usize;
    if data.is_empty() {
        // 空剪贴板：只发 start（total_len=0）+ end（全零摘要）。
        out.push(ControlMsg::ClipEnd(lp::msg::ClipboardEnd {
            sha256: [0u8; 32],
        }));
        return Ok(out);
    }
    while off < data.len() {
        let chunk = &data[off..(off + CLIP_CHUNK_CAP).min(data.len())];
        out.push(ControlMsg::ClipChunk(lp::msg::ClipboardChunk {
            seq,
            data: chunk.to_vec(),
        }));
        seq += 1;
        off += chunk.len();
    }
    out.push(ControlMsg::ClipEnd(lp::msg::ClipboardEnd {
        sha256: clip_sha256(data),
    }));
    Ok(out)
}

/// 文本/字节的 SHA-256（剪贴板末段校验，FR-07.2）。
pub fn clip_sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

// ---------- 会话状态视图（喂前端 Session 页 + 状态栏，FR-09.3）----------

/// 会话目标（前端 `SessionTarget` 的 camelCase 视图）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTargetView {
    pub name: String,
    pub id: String,
    pub res: Option<String>,
}

/// 会话状态快照（前端 `SessionState` 枚举码 + 展示标签 + 度量，喂 `Session.vue` 底部状态栏 / 顶部标签）。
///
/// 由宿主（ui-server REST / Tauri 命令）在**建立成功 / 收到事件 / 断开**时构造：
/// `state`/`state_code` 走 [`session_state_code`]，度量读 [`landesk_ui::StatusBarModel`]。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatusView {
    /// 会话状态展示标签（中文，`state_label`）。
    pub state: String,
    /// 会话状态枚举码（前端 `SessionState`：`idle`/`auth`/`running`/…）。
    pub state_code: String,
    /// 连接方式展示标签（`内网`/`P2P`/`中继`；未连为空串）。
    pub conn_mode: String,
    /// 连接方式枚举码（前端 `ConnMode`：`inner`/`p2p`/`relay`；未连为 `inner` 兜底）。
    pub conn_mode_code: String,
    /// 当前编码器展示标签（`QSV`/`NVENC`/…；未上报为空串）。
    pub encoder: String,
    /// 当前目标（未连为 `None`）。
    pub target: Option<SessionTargetView>,
    /// 端到端延迟（ms；未测得为 `None`）。
    pub latency_ms: Option<u32>,
    /// 视频丢包率（%；未统计为 `None`）。
    pub loss_pct: Option<u8>,
    /// 码率（Mbps；前端按 MB/s 展示，`None` = 未上报）。
    pub bitrate_mbps: Option<f32>,
    /// 累计编码器重置次数（FR-04.6）。
    pub reset: u32,
    // ---------- 重连进度（FR-05.4；仅 `reconnecting` 态填充，其余态保持 0/空/false）----------
    /// 当前重连尝试序号（1 起；非重连态 = 0）。
    pub reconnect_attempt: u32,
    /// 最多重试次数（默认 5；非重连态 = 0）。
    pub reconnect_max: u32,
    /// 本轮前应等待的退避秒数（1/2/4/8/16；非重连态 = 0）。
    pub reconnect_wait_secs: u64,
    /// 是否已耗尽（5 次全失败 → `true`，会话转 [`SessionState::NoPath`]）。
    pub reconnect_exhausted: bool,
    /// 重连进度展示文案（中文，喂前端浮层；非重连态为空串）。
    pub reconnect_label: String,
}

impl SessionStatusView {
    /// 核心构造：只设状态/连接方式/编码器三项（标签 + 枚举码一次算好）；
    /// 度量（目标/延迟/丢包/码率/重置）用下列 builder 按需追加，未追加保持默认（未上报）。
    /// `bitrate_kbps` 换算 Mbps 存 `bitrate_mbps`。
    pub fn new(state: &SessionState, mode: ConnMode, encoder: &str) -> Self {
        use crate::status::conn_mode_label;
        Self {
            state: crate::status::state_label(state).to_string(),
            state_code: session_state_code(state).to_string(),
            conn_mode: conn_mode_label(mode).to_string(),
            conn_mode_code: conn_mode_code(mode).to_string(),
            encoder: encoder.to_string(),
            ..Default::default()
        }
    }

    pub fn with_target(mut self, target: Option<SessionTargetView>) -> Self {
        self.target = target;
        self
    }

    pub fn with_latency_ms(mut self, latency_ms: Option<u32>) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    pub fn with_loss_pct(mut self, loss_pct: Option<u8>) -> Self {
        self.loss_pct = loss_pct;
        self
    }

    /// 码率换算（`bitrate_kbps` → `bitrate_mbps`；`None` 表示未上报）。
    pub fn with_bitrate_kbps(mut self, bitrate_kbps: Option<u32>) -> Self {
        self.bitrate_mbps = bitrate_kbps.map(|k| k as f32 / 1000.0);
        self
    }

    pub fn with_reset(mut self, reset: u32) -> Self {
        self.reset = reset;
        self
    }

    /// 重连进度（FR-05.4）：非重连 / 非无路径态传 `None`（保持默认 `0/0/0/false/""`）；
    /// `reconnecting` 态传 [`crate::reconnect_view::view_of`] 的视图（下次重试序号/退避/文案），
    /// `nopath`（已耗尽）态传 [`crate::reconnect_view::exhausted_view`]（`exhausted=true`）。
    pub fn with_reconnect(mut self, view: Option<crate::reconnect_view::ReconnectView>) -> Self {
        if let Some(v) = view {
            self.reconnect_attempt = v.attempt;
            self.reconnect_max = v.max_attempts;
            self.reconnect_wait_secs = v.wait_secs;
            self.reconnect_exhausted = v.exhausted;
            self.reconnect_label = v.label;
        }
        self
    }
}

/// 会话状态 → 前端枚举码（`ui/frontend` `SessionState` 联合类型，全小写）。
pub fn session_state_code(s: &SessionState) -> &'static str {
    match s {
        SessionState::Idle => "idle",
        SessionState::Auth => "auth",
        SessionState::Running => "running",
        SessionState::Reconnecting => "reconnecting",
        SessionState::AuthFailed => "authfailed",
        SessionState::NoPath => "nopath",
        SessionState::Closed => "closed",
    }
}

/// 连接方式 → 前端枚举码（`ui/frontend` `ConnMode`：`inner`/`p2p`/`relay`）。
pub fn conn_mode_code(m: ConnMode) -> &'static str {
    match m {
        ConnMode::Inner => "inner",
        ConnMode::P2p => "p2p",
        ConnMode::Relay => "relay",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lp::keycodes::Lkc;

    #[test]
    fn mouse_key_display_expand_single_frame() {
        let a = ControlAction::MouseMove {
            x: 100,
            y: 200,
            buttons: 1,
            mods: MOD_CTRL,
        };
        let v = expand(&a).unwrap();
        assert_eq!(v.len(), 1);
        assert!(matches!(
            &v[0],
            ControlMsg::MouseMove(MouseMove {
                x: 100,
                y: 200,
                buttons: 1,
                mods: 1
            })
        ));

        let a = ControlAction::KeyDown {
            key: Lkc::A.as_u32(),
            mods: 0,
        };
        assert!(matches!(
            expand(&a).unwrap()[0],
            ControlMsg::KeyDown(KeyEvent {
                key: 0x10,
                mods: 0
            })
        ));

        assert!(matches!(
            expand(&ControlAction::DisplaySwitch { index: 1 }).unwrap()[0],
            ControlMsg::DisplaySwitch(_)
        ));
        assert!(matches!(
            expand(&ControlAction::Heartbeat { code: 0x5A }).unwrap()[0],
            ControlMsg::Heartbeat(0x5A)
        ));
    }

    #[test]
    fn clipboard_small_text_is_three_frames() {
        // "hi" = 2B < 32KB → ClipStart + 1×ClipChunk + ClipEnd。
        let msgs = clip_text_to_msgs(b"hi").unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(matches!(msgs[0], ControlMsg::ClipStart(ref s) if s.total_len == 2));
        assert!(matches!(msgs[1], ControlMsg::ClipChunk(ref c) if c.seq == 0 && c.data == *b"hi"));
        assert!(matches!(msgs[2], ControlMsg::ClipEnd(ref e) if e.sha256 == clip_sha256(b"hi")));
    }

    #[test]
    fn clipboard_empty_is_start_and_zero_end() {
        let msgs = clip_text_to_msgs(b"").unwrap();
        assert_eq!(msgs.len(), 2, "空剪贴板只 start+end");
        assert!(matches!(msgs[0], ControlMsg::ClipStart(ref s) if s.total_len == 0));
        assert!(matches!(msgs[1], ControlMsg::ClipEnd(ref e) if e.sha256 == [0u8; 32]));
    }

    #[test]
    fn clipboard_chunks_split_at_cap() {
        // 70_000 B > 32_768（CLIP_CHUNK_CAP）→ 分 3 段（32768+32768+4464）。
        let data = vec![0xABu8; 70_000];
        let msgs = clip_text_to_msgs(&data).unwrap();
        // start + 3 chunk + end = 5 帧。
        assert_eq!(msgs.len(), 5, "start + 3 段 + end");
        let chunks: Vec<&lp::msg::ClipboardChunk> = msgs
            .iter()
            .filter_map(|m| if let ControlMsg::ClipChunk(c) = m { Some(c) } else { None })
            .collect();
        assert_eq!(chunks[0].seq, 0);
        assert_eq!(chunks[0].data.len(), CLIP_CHUNK_CAP);
        assert_eq!(chunks[1].data.len(), CLIP_CHUNK_CAP);
        assert_eq!(chunks[2].data.len(), 70_000 - 2 * CLIP_CHUNK_CAP);
        let end = msgs.last().unwrap();
        assert!(matches!(end, ControlMsg::ClipEnd(ref e) if e.sha256 == clip_sha256(&data)));
    }

    #[test]
    fn clipboard_over_cap_rejected() {
        let big = vec![0u8; (CLIP_TOTAL_CAP as usize) + 1];
        assert!(matches!(clip_text_to_msgs(&big), Err(ExpandError::ClipTooLarge(_, _))));
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc") 已知摘要（FIPS 180-4 测试向量），校验未用错哈希。
        let got = clip_sha256(b"abc");
        let hex: String = got.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn status_view_fold_and_codes() {
        use landesk_core::SessionState as SS;
        let v = SessionStatusView::new(&SS::Running, ConnMode::Relay, "QSV")
            .with_target(Some(SessionTargetView {
                name: "客厅-iMac".into(),
                id: "41C98B7E".into(),
                res: Some("2560×1440".into()),
            }))
            .with_latency_ms(Some(18))
            .with_loss_pct(Some(0))
            .with_bitrate_kbps(Some(4200))
            .with_reset(1);
        assert_eq!(v.state, "运行中");
        assert_eq!(v.state_code, "running", "前端枚举码全小写");
        assert_eq!(v.conn_mode, "中继");
        assert_eq!(v.conn_mode_code, "relay", "前端 ConnMode 枚举码");
        assert_eq!(v.encoder, "QSV");
        assert_eq!(v.latency_ms, Some(18));
        assert_eq!(v.bitrate_mbps, Some(4.2), "4200kbps → 4.2Mbps");
        assert_eq!(v.reset, 1);

        // serde camelCase 键。
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["stateCode"], "running");
        assert_eq!(json["connModeCode"], "relay");
        assert_eq!(json["target"]["res"], "2560×1440");
    }

    #[test]
    fn state_code_all_seven() {
        use landesk_core::SessionState as SS;
        assert_eq!(session_state_code(&SS::Idle), "idle");
        assert_eq!(session_state_code(&SS::Auth), "auth");
        assert_eq!(session_state_code(&SS::Running), "running");
        assert_eq!(session_state_code(&SS::Reconnecting), "reconnecting");
        assert_eq!(session_state_code(&SS::AuthFailed), "authfailed");
        assert_eq!(session_state_code(&SS::NoPath), "nopath");
        assert_eq!(session_state_code(&SS::Closed), "closed");
    }

    /// `ControlAction` 序列化形状锁定（前端 `ControlAction` 联合类型据此对齐，P1.3 双宿主归一）：
    /// `#[serde(tag="type")]` + `rename_all=camelCase` → 各变体带 `"type"` 判别 + 小驼峰字段。
    /// 改任一字段名 / 变体名 / 判别标签须与 `ui/frontend/src/lib/landesk.ts` 的 `ControlAction` 同步。
    #[test]
    fn control_action_serializes_tagged_camel_case() {
        let a = ControlAction::MouseMove {
            x: 100,
            y: 200,
            buttons: 1,
            mods: 0,
        };
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["type"], "mouseMove", "变体名 camelCase 作判别标签");
        assert_eq!(json["x"], 100);
        assert_eq!(json["y"], 200);

        let k = ControlAction::KeyDown { key: 0x10, mods: MOD_CTRL };
        let json = serde_json::to_value(&k).unwrap();
        assert_eq!(json["type"], "keyDown");
        assert_eq!(json["key"], 0x10);
        assert_eq!(json["mods"], 1);

        let c = ControlAction::ClipboardText {
            text: "hi".into(),
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["type"], "clipboardText");
        assert_eq!(json["text"], "hi");

        // 反序列化还原（前端 → 后端同形）。
        let back: ControlAction = serde_json::from_value(json).unwrap();
        assert!(matches!(back, ControlAction::ClipboardText { .. }));
    }
}
