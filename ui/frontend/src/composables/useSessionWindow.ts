// 会话窗体发起桥：把「发起远程控制 / 连接设备」路由到正确的会话宿主。
//  - Tauri 桌面：打开/聚焦**独立 OS 窗体**（label "session"，custom 窗体 + 完整 最小化/最大化/关闭），
//    目标经后端 `session_window_open` 存壳 + `session://open` 事件送达会话窗，主窗停在当前页。
//  - 浏览器预览（无 OS 窗）：退回**单页沉浸 session 视图**（`useUi.startSession` 切 mainView），保证可演示。
import { ref } from 'vue'
import { useUi } from './useUi'
import { isDesktopHost, landesk } from '../lib/landesk'
import type { SessionTarget, ConnMode } from '../types/index'

/** 最近连接条目（真实发起过的目标；本机 localStorage 持久化，非 mock）。 */
export interface RecentTarget {
  /** 设备码（快速连接输入 / 设备卡 id）。 */
  code: string
  /** 目标名称（设备卡名；快速连接时同码）。 */
  name: string
  /** 最近一次发起时间戳（ms）。 */
  at: number
}

const RECENT_KEY = 'ld-recent-targets'
const RECENT_MAX = 8
/** 最近连接列表（按最近发起倒序；未加载前为空，页面 onMounted 调 loadRecentTargets）。 */
export const recentTargets = ref<RecentTarget[]>([])

/** 从本机存储加载最近连接（坏数据/隐私模式一律空表，不影响会话主流程）。 */
export function loadRecentTargets(): void {
  try {
    const raw = localStorage.getItem(RECENT_KEY)
    const parsed: unknown = raw ? JSON.parse(raw) : []
    recentTargets.value = Array.isArray(parsed)
      ? (parsed as RecentTarget[])
        .filter((r): r is RecentTarget =>
          !!r && typeof r.code === 'string' && !!r.code && typeof r.name === 'string' && typeof r.at === 'number')
        .sort((a, b) => b.at - a.at)
        .slice(0, RECENT_MAX)
      : []
  } catch {
    recentTargets.value = []
  }
}

/** 记录一次真实发起：同码去重并置顶，最多保留 RECENT_MAX 条。 */
function recordRecent(target: SessionTarget | null): void {
  const code = target?.id?.replace(/\s/g, '')
  if (!code) return
  const rest = recentTargets.value.filter((r) => r.code !== code)
  recentTargets.value = [{ code, name: target!.name || code, at: Date.now() }, ...rest].slice(0, RECENT_MAX)
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(recentTargets.value)) } catch { /* 隐私模式：仅内存态 */ }
}

/** 发起一次远程会话：桌面开独立窗体，浏览器开单页沉浸视图。 */
export function launchSession(target: SessionTarget | null, path: ConnMode = 'inner'): void {
  recordRecent(target)
  if (isDesktopHost()) {
    // Tauri 轨：后端建/聚焦会话 OS 窗 + 送达目标（主窗不切页）。target 可为 null（会话窗内手动连）。
    void landesk.sessionWindowOpen(target, path)
    return
  }
  // 浏览器预览轨：单页沉浸 session（保留原演示路径）。
  useUi().startSession(target, path)
}

/** 读当前会话窗发起请求（完整 payload：`target` + `path`）。
 *  会话 OS 窗装载首帧调一次（后端 `session_window_target`）；浏览器轨回 `{ target: null, path: null }`。
 *  `path` 为原始串（`inner`/`p2p`/`relay`/`null`），由消费方 `asPath` 归一为 `ConnMode`。 */
export function readSessionLaunch(): Promise<{ target: SessionTarget | null; path: string | null }> {
  return landesk.sessionWindowTarget()
}

/** @deprecated 用 {@link readSessionLaunch}（带 `path`）。保留兼容：仅取 `target`。 */
export function readSessionTarget(): Promise<SessionTarget | null> {
  return landesk.sessionWindowTarget().then((p) => p.target)
}

/** 监听「会话窗被再次发起」事件（同一会话窗已开着时切换目标）。返回解绑函数。 */
export function watchSessionOpen(onTarget: (t: { target: SessionTarget | null; path: string | null }) => void): () => void {
  if (!isDesktopHost()) return () => {}
  let un: (() => void) | undefined
  landesk.sessionWindowListen(onTarget).then((fn) => { un = fn }).catch(() => {})
  return () => { if (un) un() }
}

/** 监听会话窗原生关闭请求（macOS 交通灯 ×）：壳拦截后发本事件，会话页弹二次确认。
 *  浏览器预览轨无 OS 窗 → no-op。返回解绑函数。 */
export function watchSessionCloseRequest(onClose: () => void): () => void {
  if (!isDesktopHost()) return () => {}
  let un: (() => void) | undefined
  landesk.sessionWindowListenClose(onClose).then((fn) => { un = fn }).catch(() => {})
  return () => { if (un) un() }
}
