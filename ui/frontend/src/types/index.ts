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

/** 受控端配置（对齐 landesk-core/Config） */
export interface LanDeskConfig {
  device_name: string
  password: string
  preset: Preset
  dirty_rect: boolean
  mdns_enabled: boolean
  signaling_enabled: boolean
  signaling_url: string
  relay_enabled: boolean
  relay_url: string
  stun_enabled: boolean
  stun_url: string
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
