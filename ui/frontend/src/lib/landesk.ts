// LanDesk 前端 ↔ 后端桥接封装（对齐 ui/src-tauri 的 Tauri 命令 + apps/ui-server 的 REST）。
// 双轨降级：Tauri 宿主走 invoke；浏览器走 REST /api/*。
// 出参为 landesk-ui 的可序列化模型（JSON 直进组件）。
import type { ConnMode, DeviceRow, LanDeskConfig, LocalInfo, VideoFrameMsg } from '../types/index'
import type { ControlAction, SessionResp } from '../types/index'

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

/** 重连退避计划（对齐后端 `ui::ReconnectSchedule`，P1.2「会话命令面」，喂 Session 重连浮层）。 */
export interface ReconnectSchedule {
  max_attempts: number
  /** 第 N 次重试前应等待的秒数（N 从 1 起，长度 = max_attempts；默认 1/2/4/8/16）。 */
  waits: number[]
  label: string
}

/** 连接路径决策（对齐后端 `ui::PathDecisionView`，双宿主同形，P1.3 归一）：
 *  - 有路径：`{ connMode: 'inner'|'p2p'|'relay' }`
 *  - 无路径：`{ nopath: true }`
 *  `connMode` 小写与 `ConnMode` 对齐；消费方读 `dec.connMode`（无则 `dec.nopath` 为真）。 */
export interface PathDecision {
  connMode?: ConnMode
  nopath?: boolean
}

function isTauri(): boolean {
  return typeof window !== 'undefined' &&
    !!window.__TAURI__?.core?.invoke
}

/** 当前是否桌面（Tauri）宿主——同步判定（读 `window.__TAURI__`）。
 *  自绘窗口控制按钮仅在桌面宿主有意义（浏览器预览无窗口可控，隐藏按钮 + 拖拽区 no-op）。 */
export function isDesktopHost(): boolean {
  return isTauri()
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

  /** 设备列表（FR-01.2/01.4，P1.1 下沉）。
   *  - Tauri：壳内 `AppState.devices`（后台 mDNS 驱动）自维护，命令**无入参**，直接出当前表。
   *  - REST：`/api/devices`（ui-server 共享 `DeviceTable`，后台镜像监听器在线表）。
   *  入参 `devices`/`nowEpochSecs` 仅兼容旧 REST 调用面，Tauri 轨忽略。 */
  deviceList(_devices: RawDevice[] = [], _nowEpochSecs: number = 0): Promise<RawDevice[]> {
    return isTauri()
      ? tauriInvoke<RawDevice[]>('device_list')
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

  /** 重连退避计划（P1.2「会话命令面」，FR-05.4）：完整 1/2/4/8/16s 序列 + 次数，喂 Session 重连浮层。 */
  reconnectSchedule(): Promise<ReconnectSchedule> {
    return isTauri()
      ? tauriInvoke<ReconnectSchedule>('reconnect_schedule')
      : rest<ReconnectSchedule>('/api/reconnect-schedule')
  },

  /** 本机信息（P2.8「本机信息 API」，矩阵 §7.5）：设备码/临时密码/能力/在线态。
   *  - Tauri：壳 `local_info` 命令（设备表驱动 mDNS/在线态，设备码 FNV 派生）。
   *  - REST：`/api/local-info`。 */
  localInfo(): Promise<LocalInfo> {
    return isTauri()
      ? tauriInvoke<LocalInfo>('local_info')
      : rest<LocalInfo>('/api/local-info')
  },

  /** 日志查询（P2.8「日志查询 API」，矩阵 §7.5）：环形缓冲 + 级别/关键字过滤。
   * `level`/`keyword` 可空（`ALL`/空 = 不限）。后端出参 `t` 为 epoch 微秒。 */
  logQuery(level?: string, keyword?: string): Promise<RawLogLine[]> {
    const q = new URLSearchParams()
    if (level) q.set('level', level)
    if (keyword) q.set('q', keyword)
    const qs = q.toString()
    return isTauri()
      ? tauriInvoke<RawLogLine[]>('log_query', { level: level ?? null, keyword: keyword ?? null })
      : rest<RawLogLine[]>(`/api/logs${qs ? `?${qs}` : ''}`)
  },

  /** 日志清空（P2.8「日志清空 API」，矩阵 §7.5）。 */
  logClear(): Promise<void> {
    return isTauri()
      ? tauriInvoke<void>('log_clear')
      : rest<void>('/api/logs/clear', { method: 'POST' })
  },

  // ---------- 会话命令面（P2 遗留 ①，矩阵 §7.5「会话建立/鉴权/断开 + 画面/键鼠/剪贴板/截图/显示器」）----------
  // 出参统一 `SessionResp`（双宿主同形 P1.3）：成功带 `status`/`sent`，失败带 `error`（前端读 `error` 区分）。
  //  - Tauri：`session_connect`/`session_send_control`/`session_status`/`session_close` 命令（异步+State 返回 Result，invoke 拆出 `SessionResp`）。
  //  - REST：`/api/session/connect`/`control`/`status`/`close`（ui-server 共享 `SessionManager`，恒 200，读 `error`）。

  /** 会话建立（FR-01 一键连接 + 鉴权）：`deviceId` 经后端 `DeviceTable` 解析 `ip:port`，
   *  QUIC 拨号 + bootstrap 隧道 + ECDH 会话升级。`sni` 缺省 `"localhost"`（环回）。 */
  sessionConnect(deviceId: string, password: string, sni?: string): Promise<SessionResp> {
    return isTauri()
      ? tauriInvoke<SessionResp>('session_connect', { deviceId, password, sni: sni ?? null })
      : rest<SessionResp>('/api/session/connect', {
          method: 'POST',
          body: { deviceId, password, sni: sni ?? null },
        })
  },

  /** 发送控制帧（FR-06 键鼠 / FR-07 剪贴板 / FR-04.5 编码 / FR-03.3 显示器）：
   *  一条 `ControlAction` 意图展开成 1..N 线层帧逐帧发出（剪贴板含 32KB 分段 + SHA-256）。
   *  出参 `sent` = 本次发出帧数；未建会话 / 剪贴板超限带 `error`。 */
  sessionSendControl(action: ControlAction): Promise<SessionResp> {
    return isTauri()
      ? tauriInvoke<SessionResp>('session_send_control', { action })
      : rest<SessionResp>('/api/session/control', { method: 'POST', body: { action } })
  },

  /** 会话状态（FR-09.3）：出当前状态机 + 折叠快照（未连 = `idle`、无目标、度量为 `null`）。 */
  sessionStatus(): Promise<SessionResp> {
    return isTauri()
      ? tauriInvoke<SessionResp>('session_status')
      : rest<SessionResp>('/api/session/status')
  },

  /** 会话关闭（FR-02.4）：优雅断开 QUIC + 状态机落 `Closed`。 */
  sessionClose(): Promise<SessionResp> {
    return isTauri()
      ? tauriInvoke<SessionResp>('session_close')
      : rest<SessionResp>('/api/session/close', { method: 'POST' })
  },

  // ---------- 视频数据面（P2 遗留 ②，矩阵 §7.5「画面」）----------
  // 画面帧经 `VideoDgram` 分片 → QUIC datagram → 控制端 `JitterBuffer` 重组为完整 NALU，
  // 由宿主逐帧送达前端（出参 `VideoFrameMsg`，`nalu` 为 base64 裸 H.264 字节）：
  //  - Tauri 桌面：`session_video_poll` **同步拉取**（`try_recv` 排空本轮 `Vec<VideoFrameMsg>`，
  //    前端 ~33ms 轮询一次；非 live / 未建会话返回空数组，不抛错）。
  //  - 浏览器 REST：`GET /api/session/video` **SSE 推流**（`EventSource`，逐帧 `data:` 事件）。
  // 两种传输在 `useLanDesk.startVideoStream` 里按宿主分流（Tauri→poll / 浏览器→SSE），上层无感。

  /** Tauri 宿主：同步拉取自上次以来新到的画面帧（排空本轮队列）。
   *  非 Tauri 宿主返回空数组（浏览器轨走 `sessionVideoStreamUrl` + `EventSource`）。 */
  sessionVideoPoll(): Promise<VideoFrameMsg[]> {
    return isTauri()
      ? tauriInvoke<VideoFrameMsg[]>('session_video_poll')
      : Promise.resolve([])
  },

  /** 浏览器宿主：返回 SSE 画面流端点 URL（相对路径，与 ui-server 同域）。
   *  `EventSource(url)` 逐帧推送 `VideoFrameMsg` JSON；无活跃会话时服务端回 404。
   *  Tauri 宿主走 `sessionVideoPoll`，此 URL 不会被消费。 */
  sessionVideoStreamUrl(): string {
    return '/api/session/video'
  },

  // ---------- 会话独立 OS 窗体（custom 窗体，完整 最小化/最大化/关闭）----------
  // 发起远程控制/连接时由主窗体调 `sessionWindowOpen`：后端建/聚焦 label "session" 的
  // 独立窗体并送达目标；会话窗体自身装载时 `sessionWindowTarget` 取目标、
  // `sessionWindowListen` 订阅「再次发起」事件（同一会话窗已开着时切目标）。
  // 浏览器轨（ui-server REST 无 OS 窗概念）：`sessionWindowOpen` no-op（浏览器预览走单页
  // session 视图，由 `useSessionWindow` 分流），`sessionWindowTarget`/`Listen` 回默认。

  /** Tauri：建/聚焦独立会话 OS 窗体并送达目标。浏览器轨 no-op。 */
  sessionWindowOpen(target: { name: string; id: string; res?: string } | null, path?: string): Promise<void> {
    return isTauri()
      ? tauriInvoke<void>('session_window_open', { target, path: path ?? null })
      : Promise.resolve()
  },

  /** Tauri：取当前会话窗目标（会话窗体装载时首帧）。出完整 payload（`target` + `path`）。
   *  浏览器轨回 `{ target: null, path: null }`。 */
  sessionWindowTarget(): Promise<{ target: { name: string; id: string; res?: string } | null; path: string | null }> {
    return isTauri()
      ? tauriInvoke<{ target: { name: string; id: string; res?: string } | null; path: string | null }>('session_window_target')
      : Promise.resolve({ target: null, path: null })
  },

  /** Tauri：订阅 `session://open` 事件（同一会话窗已开时切换目标），返回解绑函数。
   *  浏览器轨返回 `Promise.resolve(() => {})`（no-op 解绑）。 */
  sessionWindowListen(
    onTarget: (t: { target: { name: string; id: string; res?: string } | null; path: string | null }) => void,
  ): Promise<() => void> {
    if (!isTauri()) return Promise.resolve(() => {})
    // 会话窗体自身的 Tauri 事件通道（withGlobalTauri 注入 `__TAURI__.event.listen`）。
    const listen = window.__TAURI__?.event?.listen
    if (!listen) return Promise.resolve(() => {})
    return listen('session://open', (ev) => onTarget(ev.payload as { target: { name: string; id: string; res?: string } | null; path: string | null }))
      .catch(() => () => {})
  },

  // ---------- 自绘主窗体（去系统标题栏，跨平台窗口控制）----------
  // `decorations:false`（Windows/Linux）或 `titleBarStyle:overlay`（macOS 保留交通灯）下，
  // 系统不再画窗口三键，改由前端 TitleBar 自绘按钮调这些命令。平台分流：
  //  - Tauri 桌面：`window_platform`（壳读 `std::env::consts::OS`）出当前目标 OS；窗口控制走 `invoke`。
  //  - 浏览器预览：`navigator` 启发式判平台；窗口控制 no-op（浏览器无自绘窗口可控制）。

  /** 当前目标平台（`'macos' | 'windows' | 'linux'`）。
   *  Tauri 轨走 `window_platform` 命令（编译期目标 OS）；浏览器轨用 `navigator` 启发式兜底
   *  （`navigator.platform` 已废弃，仅作预览态判 macOS/Windows/Linux，桌面壳恒走 Tauri 轨）。 */
  platform(): Promise<'macos' | 'windows' | 'linux'> {
    if (isTauri()) {
      return tauriInvoke<'macos' | 'windows' | 'linux'>('window_platform')
    }
    return Promise.resolve(detectBrowserPlatform())
  },

  /** 切换最大化 / 还原（返回是否已最大化）。浏览器轨 no-op（返 `false`）。 */
  windowToggleMaximize(): Promise<boolean> {
    return isTauri()
      ? tauriInvoke<boolean>('window_toggle_maximize')
      : Promise.resolve(false)
  },

  /** 最小化。浏览器轨 no-op。 */
  windowMinimize(): Promise<void> {
    return isTauri()
      ? tauriInvoke<void>('window_minimize')
      : Promise.resolve()
  },

  /** 关闭窗口。浏览器轨 no-op（预览态不销毁宿主）。 */
  windowClose(): Promise<void> {
    return isTauri()
      ? tauriInvoke<void>('window_close')
      : Promise.resolve()
  },
}

/** 浏览器宿主（非 Tauri）平台启发式：`navigator.platform` / `userAgent` 判 macOS/Windows/Linux。
 *  仅作预览态兜底（桌面壳恒走 Tauri 轨的 `window_platform`），不保证精确——
 *  浏览器里窗口控制按钮本就 no-op，平台只影响「是否画按钮」的预览观感。 */
function detectBrowserPlatform(): 'macos' | 'windows' | 'linux' {
  if (typeof navigator === 'undefined') return 'linux'
  const ua = `${navigator.userAgent} ${navigator.platform}`.toLowerCase()
  if (ua.includes('mac') || ua.includes('darwin')) return 'macos'
  if (ua.includes('win')) return 'windows'
  return 'linux'
}

/** 后端 `ui::LogLine` 序列化形状（`t` 为 epoch 微秒 `number`，`mod` 键名；camelCase）。 */
export interface RawLogLine {
  t: number
  level: string
  mod: string
  msg: string
  kv: string
}

export type { DeviceRow }
export default landesk
