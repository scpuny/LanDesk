// LanDesk 前端共享类型声明。
// 与后端可序列化模型对齐：crates/ui/src/{status,devices,config_store}.rs。

/** 设备类型（用于选择图标） */
export type DeviceKind = 'monitor' | 'laptop' | 'chip' | 'server' | 'phone'

/** 设备列表行（对齐 DeviceRow，前端展示层补充 kind / online） */
export interface DeviceRow {
  id: string
  name: string
  addr: string
  res: string
  enc: string[]
  relay: boolean
  online: boolean
  kind: DeviceKind
  /** 离线分钟数，0 表示在线/刚刚 */
  lastSeen: number
}

/** 会话状态机（对齐 landesk-core/session 状态 + V2 设计稿 7 态） */
export type SessionState =
  | 'idle'
  | 'auth'
  | 'running'
  | 'reconnecting'
  | 'authfailed'
  | 'nopath'
  | 'closed'

/** 连接路径 */
export type ConnMode = 'inner' | 'p2p' | 'relay'

/** 会话质量指标（对齐 StatusBarModel） */
export interface SessionMetrics {
  encoder: string
  latency: number
  loss: number
  bitrate: number
  reset: number
}

/** 当前连接目标 */
export interface SessionTarget {
  name: string
  id: string
  res?: string
}

/** 画质预设：0=流畅 1=平衡 2=高清 */
export type Preset = 0 | 1 | 2

// ---------- 会话命令面（P2 遗留 ①，矩阵 §7.5，对齐 `ui::manager::ControlAction`/`SessionResp`）----------
/** 控制意图（前端工具栏/快捷键抽象，`camelCase` `type` 判别 + 字段对齐后端 `ControlAction`）。
 *  由 `expand` 展开成 1..N 线层帧；剪贴板含 32KB 分段 + SHA-256。键码 `key` 为 LKC u32
 *  （A=0x10 / Digit0=0x30 / 方向 0x58-0x5B / F1-F12 0x62-0x6D，见 `docs/protocol.md` §1.4）。 */
export type ControlAction =
  | { type: 'mouseMove'; x: number; y: number; buttons: number; mods: number }
  | { type: 'mouseButton'; button: number; down: number }
  | { type: 'mouseWheel'; dx: number; dy: number }
  | { type: 'keyDown'; key: number; mods: number }
  | { type: 'keyUp'; key: number; mods: number }
  | { type: 'clipboardText'; text: string }
  | { type: 'encodeParams'; bitrateKbps: number; keyframeInterval: number; preset: number }
  | { type: 'displaySwitch'; index: number }
  | { type: 'heartbeat'; code: number }
  | { type: 'ping'; ts: number }
  | { type: 'close'; reason: number }

/** 键盘修饰键位（对齐 `lp::keycodes` `MOD_*`，FR-06.2）。 */
export const MOD_CTRL = 0b0001
export const MOD_SHIFT = 0b0010
export const MOD_ALT = 0b0100
export const MOD_META = 0b1000

/** 会话目标（对齐 `ui::session::SessionTargetView`）。 */
export interface SessionTargetView {
  name: string
  id: string
  res?: string
}

/** 会话状态快照（对齐 `ui::session::SessionStatusView`，`camelCase`）。 */
export interface SessionStatus {
  /** 状态展示标签（中文，`state_label`）。 */
  state: string
  /** 状态枚举码（前端 `SessionState`：`idle`/`auth`/`running`/…）。 */
  stateCode: SessionState
  /** 连接方式展示标签（`内网`/`P2P`/`中继`；未连为空串）。 */
  connMode: string
  /** 连接方式枚举码（`inner`/`p2p`/`relay`；未连兜底 `inner`）。 */
  connModeCode: ConnMode
  /** 当前编码器展示标签（`QSV`/`NVENC`/…；未上报为空串）。 */
  encoder: string
  /** 当前目标（未连为 `null`）。 */
  target: SessionTargetView | null
  /** 端到端延迟（ms；未测得为 `null`）。 */
  latencyMs: number | null
  /** 视频丢包率（%；未统计为 `null`）。 */
  lossPct: number | null
  /** 码率（Mbps；未上报为 `null`）。 */
  bitrateMbps: number | null
  /** 累计编码器重置次数（FR-04.6）。 */
  reset: number
}

/** 会话命令统一出参（对齐 `ui::manager::SessionResp`，双宿主同形 P1.3）。
 *  成功带 `status`（+ 可选 `sent` 帧数）；失败 `error` 给可读原因（`status`/`sent` 可能仍带最新快照）。 */
export interface SessionResp {
  status: SessionStatus | null
  /** 本次发出帧数（仅 `sendControl` 成功；其余 `null`）。 */
  sent: number | null
  /** 失败原因（成功为 `null`/缺省）。 */
  error?: string | null
}

/** 日志级别 */
export type LogLevel = 'INFO' | 'WARN' | 'ERROR' | 'DEBUG'
export type LogFilter = LogLevel | 'ALL'

/** 单行日志 */
export interface LogLine {
  t: string
  level: LogLevel
  mod: string
  msg: string
  kv: string
}

/** 受控端配置（对齐 landesk-core/Config）。
 *  后端 `Option<String>` 字段序列化可为 `null`，故此处允许 `string | null`；
 *  `preset` 为整数码率档 `0|1|2`（EncodePreset serde 按 u8 判别值序列化）；
 *  `relay_url` 已对齐后端（`#[serde(default)]` `Option<String>`）。
 *  `stun_enabled` 为前端展示态（后端 STUN 以 `stun_url` 是否置位判定，无独立开关字段）。 */
export interface LanDeskConfig {
  device_name: string
  password: string | null
  preset: Preset
  dirty_rect: boolean
  mdns_enabled: boolean
  signaling_enabled: boolean
  signaling_url: string | null
  relay_enabled: boolean
  relay_url: string | null
  stun_enabled?: boolean
  stun_url: string | null
  log_level: 'Info' | 'Debug' | 'Warn' | 'Error'
}

/** 本机信息 */
export interface LocalInfo {
  name: string
  deviceCode: string
  tempPassword: string
  mdns: boolean
  signaling: string
  relay: boolean
  enc: string[]
  online: boolean
}

/** Toast 提示 */
export interface ToastItem {
  id: number
  message: string
  tone: 'success' | 'error' | 'warning' | 'info'
}

/** Toast 语气 */
export type ToastTone = ToastItem['tone']

/** 二次确认请求 */
export interface ConfirmRequest {
  title: string
  message: string
  tone: 'danger' | 'warning'
  confirmText: string
  cancelText: string
}

/** 主视图标识 */
export type MainView = 'devices' | 'session' | 'settings' | 'logs'

/** 明暗偏好 */
export type ThemePref = 'light' | 'dark' | 'system'

/** 视觉风格 id */
export type PaletteId =
  | 'prototype'
  | 'graphite'
  | 'aurora'
  | 'slate'
  | 'carbon'
  | 'nocturne'
  | 'amber'
