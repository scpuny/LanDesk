// LanDesk 前端 ↔ 后端桥接封装（对齐 ui/src-tauri 的 Tauri 命令 + apps/ui-server 的 REST）。
// 双轨降级：Tauri 宿主走 invoke；浏览器走 REST /api/*。
// 出参为 landesk-ui 的可序列化模型（JSON 直进组件）。
import type { ConnMode, DeviceRow, LanDeskConfig } from '../types/index'

/** ICE 打洞三态（对齐 `ice::HoleResult`，serde 默认按变体名序列化）。 */
export type HoleResult = 'Direct' | 'NeedRelay' | 'NoPath'

/** 重连视图（对齐后端 `ui::ReconnectView`：attempt/max_attempts/wait_secs/exhausted/label）。 */
export interface ReconnectView {
  attempt: number
  max_attempts: number
  wait_secs: number
  exhausted: boolean
  label: string
}

/** 连接路径决策（对齐后端 `ui::PathDecision` / REST `ChoosePathResp`）。
 *  `use.connMode` = `ConnMode`（`inner|p2p|relay`）；`nopath` = 拒绝连接。
 *  两宿主 JSON 形状略有差异（Tauri 裸 `PathDecision` 为 `{Use:{Inner}}`，REST 为
 *  `{use:{conn_mode}}`/`{nopath:true}`），消费方按「取 `use.connMode` 或 `nopath`」兼容读。 */
export interface PathDecision {
  use?: { connMode: ConnMode }
  nopath?: boolean
}

function isTauri(): boolean {
  return typeof window !== 'undefined' &&
    !!window.__TAURI__?.core?.invoke
}

/** 探测当前宿主是否有可用后端（决定 live vs mock 走哪条轨）。
 *  - Tauri 宿主：`withGlobalTauri` 注入的 `window.__TAURI__.core.invoke` 在 → 直接 live。
 *  - 浏览器宿主：显式注入 `window.__LANDesk_live__===true`（如嵌入方 / index.html）→ live；
 *    否则同源自检 `GET /health`（仅 ui-server 暴露且返回 `"ok"`；vite dev / 纯静态 mock
 *    无此端点或其 SPA 兜底返回 HTML 而非 `"ok"`）→ 据此判定，避免把 mock 误判为 live。 */
export async function detectLiveHost(): Promise<boolean> {
  if (typeof window === 'undefined') return false
  if (window.__TAURI__?.core?.invoke) return true
  if (window.__LANDesk_live__ === true) return true
  let live = false
  try {
    const r = await fetch('/health', { method: 'GET' })
    live = r.ok && (await r.text()) === 'ok'
  } catch {
    live = false
  }
  window.__LANDesk_live__ = live
  return live
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return await window.__TAURI__!.core!.invoke(cmd, args) as T
}

async function rest<T>(path: string, options: { method?: string; body?: unknown } = {}): Promise<T> {
  const { method = 'GET', body } = options
  const res = await fetch(path, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  })
  if (res.status === 204) return null as T
  if (!res.ok) {
    const text = await res.text().catch(() => '')
    let msg = `${res.status} ${res.statusText}`
    try {
      const j = JSON.parse(text) as { error?: string }
      msg = j.error ? `${res.status}: ${j.error}` : text
    } catch {
      if (text) msg = text
    }
    throw new Error(msg)
  }
  const ct = res.headers.get('content-type') || ''
  return (ct.includes('application/json') ? await res.json() : await res.text()) as T
}

interface RawDevice {
  id: string
  name: string
  addr: string
  res: string
  enc?: string | string[]
  relay?: boolean
  online?: boolean
  last_seen?: number
}

export const landesk = {
  /** 配置页：读或首次生成默认模板（FR-08） */
  configLoadOrInit(): Promise<LanDeskConfig> {
    return isTauri()
      ? tauriInvoke<LanDeskConfig>('config_load_or_init')
      : rest<LanDeskConfig>('/api/config')
  },

  /** 配置页：保存（FR-08） */
  configSave(config: LanDeskConfig): Promise<void> {
    return isTauri()
      ? tauriInvoke<void>('config_save', { config })
      : rest<void>('/api/config', { method: 'POST', body: config })
  },

  /** 设备列表（FR-01.2/01.4） */
  deviceList(devices: RawDevice[], nowEpochSecs: number): Promise<RawDevice[]> {
    return isTauri()
      ? tauriInvoke<RawDevice[]>('device_list', { devices, nowEpochSecs })
      : rest<RawDevice[]>('/api/devices')
  },

  /** 重连指示器（FR-05.4）。`null` = 已耗尽（前端转 NoPath）。 */
  reconnectView(usedAttempts: number): Promise<ReconnectView | null> {
    return isTauri()
      ? tauriInvoke<ReconnectView | null>('reconnect_view', { usedAttempts })
      : rest<ReconnectView | null>('/api/reconnect', { method: 'POST', body: { usedAttempts } })
  },

  /** 连接路径决策（FR-05/11）。`hole` 传 ICE 打洞三态（非 boolean，保留 Direct/NeedRelay/NoPath 语义）。 */
  choosePath(innerFound: boolean, hole: HoleResult, relayEnabled: boolean): Promise<PathDecision> {
    return isTauri()
      ? tauriInvoke<PathDecision>('choose_path', { innerFound, hole, relayEnabled })
      : rest<PathDecision>('/api/choose-path', {
          method: 'POST', body: { innerFound, hole, relayEnabled },
        })
  },
}

export type { DeviceRow }
export default landesk
