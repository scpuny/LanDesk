//! 控制流报文类型与载荷定义（对齐 `docs/protocol.md` §1.2 / §1.3）。

use serde::{Deserialize, Serialize};

/// 报文类型值（对齐 protocol.md §1.2 表格）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MsgType {
    // 鉴权
    AuthReq = 0x01,
    AuthRes = 0x02,
    // 输入（FR-06）
    MouseMove = 0x10,
    MouseButton = 0x11,
    MouseWheel = 0x12,
    KeyDown = 0x20,
    KeyUp = 0x21,
    // 剪贴板（FR-07）
    ClipStart = 0x30,
    ClipChunk = 0x31,
    ClipEnd = 0x32,
    // 编码器控制（FR-04）
    EncodeParams = 0x40,
    EncodeEvent = 0x41,
    DisplayInfo = 0x42,
    DisplaySwitch = 0x43,
    // 会话
    Ping = 0x50,
    Pong = 0x51,
    SessionClose = 0x60,
    Reconnect = 0x61,
    Heartbeat = 0x70,
}

impl MsgType {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
    /// 未知 type 的容错解析：接收端忽略未知值并 warn（对齐 protocol.md §7 演进预留）。
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x01 => Self::AuthReq,
            0x02 => Self::AuthRes,
            0x10 => Self::MouseMove,
            0x11 => Self::MouseButton,
            0x12 => Self::MouseWheel,
            0x20 => Self::KeyDown,
            0x21 => Self::KeyUp,
            0x30 => Self::ClipStart,
            0x31 => Self::ClipChunk,
            0x32 => Self::ClipEnd,
            0x40 => Self::EncodeParams,
            0x41 => Self::EncodeEvent,
            0x42 => Self::DisplayInfo,
            0x43 => Self::DisplaySwitch,
            0x50 => Self::Ping,
            0x51 => Self::Pong,
            0x60 => Self::SessionClose,
            0x61 => Self::Reconnect,
            0x70 => Self::Heartbeat,
            _ => return None,
        })
    }
}

// ---------- 鉴权 ----------

/// 握手请求（FR-02）。见 protocol.md §0.2 双向 nonce + proof 互验。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    /// 受控端设备 ID（128-bit，16B）。
    pub device_id: [u8; 16],
    pub server_nonce: [u8; 32],
    pub client_nonce: [u8; 32],
    pub client_x25519: [u8; 32],
    /// 密码证明 = SHA256(PBKDF2(password, client_nonce||server_nonce, 100_000))。
    pub proof: [u8; 32],
    /// bit0: 一次性密码模式。
    pub flags: u8,
}

/// 握手结果 / 失败原因（FR-02.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResult {
    /// 0=成功, 1=密码错误, 2=设备不存在, 3=nonce 重放, 4=OTP 已失效。
    pub code: u8,
    pub server_x25519: [u8; 32],
    pub server_nonce: [u8; 32],
}

// ---------- 输入（FR-06）----------

/// 鼠标移动。坐标归一化 0..=65535，受控端按自身分辨率还原（FR-06.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseMove {
    pub x: u16,
    pub y: u16,
    /// bit0 左 bit1 右 bit2 中。
    pub buttons: u16,
    /// bit0 Ctrl bit1 Shift bit2 Alt bit3 Meta（FR-06.2）。
    pub mods: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseButton {
    pub button: u8,
    pub down: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseWheel {
    /// 负 = 下/左。
    pub dx: i16,
    pub dy: i16,
}

/// 键盘事件。使用统一 LanDesk Key Code（LKC，见 keycodes 模块）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: u32,
    pub mods: u8,
}

// ---------- 剪贴板（FR-07，双向）----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardStart {
    pub total_len: u32,
    /// MVP 仅文本（mime_flags 预留）。
    pub mime_flags: u8,
}

/// 定长 32KB 分段，末段可短（对齐 protocol.md §1.3）。
pub const CLIP_CHUNK_CAP: usize = 32 * 1024;
/// 单份剪贴板上限 1MB（FR-07.2）。
pub const CLIP_TOTAL_CAP: u32 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardEnd {
    pub sha256: [u8; 32],
}

/// 剪贴板分段数据。载荷在 wire 层按实际长度编码（末段短），
/// 这里用 Vec 表示变长内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardChunk {
    pub seq: u16,
    pub data: Vec<u8>,
}

// ---------- 编码器控制（FR-04）----------

/// 编码器枚举（对齐 protocol.md §1.3 ENC_* 常量）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderKind {
    Qsv = 1,
    Nvenc = 2,
    Amf = 3,
    V4l2m2m = 4,
    VideoToolbox = 5,
    Openh264 = 9,
}

impl EncoderKind {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// 编码器预设（FR-08.5）。
///
/// serde 序列化为**整数码率档**（`0/1/2`，即 `#[repr(u8)]` 判别值）而非变体名：
/// 与前端 `Preset = 0|1|2`（`ui/frontend/src/types/index.ts`）对齐，`Config` 的 JSON
/// 往返 + REST/Tauri 出参才不被 `"Smooth"` 字符串坑到（见 docs/ui-backend-matrix.md §6.3）。
/// 仅走 JSON（配置持久化 / UI 出参）；二进制 `lp` 线层从不直接编码 `EncodePreset`
/// （`EncodeParams.preset` 是裸 `u8`），故此 serde 表示不影响 wire。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum EncodePreset {
    Smooth = 0,
    Balanced = 1,
    Hd = 2,
}

impl From<EncodePreset> for u8 {
    fn from(p: EncodePreset) -> Self {
        p as u8
    }
}

impl TryFrom<u8> for EncodePreset {
    /// 反序列化错误：返回无法识别的原始判别值。
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        Ok(match v {
            0 => EncodePreset::Smooth,
            1 => EncodePreset::Balanced,
            2 => EncodePreset::Hd,
            _ => return Err(v),
        })
    }
}

/// 运行时码率 / I 帧间隔 / 预设调整（FR-04.5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodeParams {
    /// 0 = 维持当前；0xFFFF = 强制立即 IDR（视频丢 I 帧分片时触发，protocol.md §2.2）。
    pub bitrate_kbps: u16,
    pub keyframe_interval: u16,
    pub preset: u8,
}

/// 编码器异常事件（FR-04.6，本地 close+reopen，不断会话）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodeEvent {
    /// 1=编码器已重置 2=降级到软编 3=分辨率变更重建。
    pub code: u8,
    pub active_encoder: u8,
}

// ---------- 显示器（FR-03.3）----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayEntry {
    pub index: u8,
    pub w: u16,
    pub h: u16,
    pub name_hash: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub count: u8,
    pub current: u8,
    pub entries: [DisplayEntry; 8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySwitch {
    pub index: u8,
}

// ---------- 会话 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    pub ts: u64,
}

/// 会话关闭原因（FR-02.4 / NFR-01）。
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseReason {
    User = 1,
    PasswordInvalid = 2,
    EncoderUnrecoverable = 3,
    Network = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClose {
    pub reason: u8,
}

/// 重连请求（FR-05.4，指数退避由 landesk-core 调度）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconnect {
    pub reason: u8,
}

/// 控制流报文的完整枚举（untagged 序列化由 wire 层按 type 字段路由）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMsg {
    AuthReq(AuthRequest),
    AuthRes(AuthResult),
    MouseMove(MouseMove),
    MouseButton(MouseButton),
    MouseWheel(MouseWheel),
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    ClipStart(ClipboardStart),
    ClipChunk(ClipboardChunk),
    ClipEnd(ClipboardEnd),
    EncodeParams(EncodeParams),
    EncodeEvent(EncodeEvent),
    DisplayInfo(DisplayInfo),
    DisplaySwitch(DisplaySwitch),
    Ping(Ping),
    Close(SessionClose),
    Reconnect(Reconnect),
    Heartbeat(u8),
}

impl ControlMsg {
    pub fn msg_type(&self) -> MsgType {
        match self {
            ControlMsg::AuthReq(_) => MsgType::AuthReq,
            ControlMsg::AuthRes(_) => MsgType::AuthRes,
            ControlMsg::MouseMove(_) => MsgType::MouseMove,
            ControlMsg::MouseButton(_) => MsgType::MouseButton,
            ControlMsg::MouseWheel(_) => MsgType::MouseWheel,
            ControlMsg::KeyDown(_) => MsgType::KeyDown,
            ControlMsg::KeyUp(_) => MsgType::KeyUp,
            ControlMsg::ClipStart(_) => MsgType::ClipStart,
            ControlMsg::ClipChunk(_) => MsgType::ClipChunk,
            ControlMsg::ClipEnd(_) => MsgType::ClipEnd,
            ControlMsg::EncodeParams(_) => MsgType::EncodeParams,
            ControlMsg::EncodeEvent(_) => MsgType::EncodeEvent,
            ControlMsg::DisplayInfo(_) => MsgType::DisplayInfo,
            ControlMsg::DisplaySwitch(_) => MsgType::DisplaySwitch,
            ControlMsg::Ping(_) => MsgType::Ping,
            ControlMsg::Close(_) => MsgType::SessionClose,
            ControlMsg::Reconnect(_) => MsgType::Reconnect,
            ControlMsg::Heartbeat(_) => MsgType::Heartbeat,
        }
    }
}
