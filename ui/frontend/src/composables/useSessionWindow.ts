// 会话窗体发起桥：把「发起远程控制 / 连接设备」路由到正确的会话宿主。
//  - Tauri 桌面：打开/聚焦**独立 OS 窗体**（label "session"，custom 窗体 + 完整 最小化/最大化/关闭），
//    目标经后端 `session_window_open` 存壳 + `session://open` 事件送达会话窗，主窗停在当前页。
//  - 浏览器预览（无 OS 窗）：退回**单页沉浸 session 视图**（`useUi.startSession` 切 mainView），保证可演示。
import { useUi } from './useUi'
import { isDesktopHost, landesk } from '../lib/landesk'
import type { SessionTarget, ConnMode } from '../types/index'

/** 发起一次远程会话：桌面开独立窗体，浏览器开单页沉浸视图。 */
export function launchSession(target: SessionTarget | null, path: ConnMode = 'inner'): void {
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
