// 自绘主窗体：窗口控制（去系统标题栏后，前端自绘 min/max/close + 平台判定）。
// 桌面（Tauri）宿主走后端 `window_*` 命令；浏览器预览 no-op + `navigator` 判平台（仅影响按钮观感）。
// 平台分流（对齐 Tauri 平台配置 `tauri.{macos,windows,linux}.conf.json`）：
//   - macOS：`decorations:true + titleBarStyle:overlay + hiddenTitle` → **保留系统原生交通灯**（左上），
//     前端**不画**窗口控制按钮，仅把顶栏左侧留出交通灯宽度（`.ld-topbar--mac` 左 padding）。
//   - Windows / Linux：`decorations:false` 全自绘 → 前端**画** min/max/close 三键（顶栏右侧）。
import { ref, computed, type Ref, type ComputedRef } from 'vue'
import { landesk, isDesktopHost } from '../lib/landesk'

export type Platform = 'macos' | 'windows' | 'linux'

/** 窗口控制 composable（模块级惰性判定，避免每处顶栏各发一次平台查询）。 */
export function useWindowControls(): {
  isDesktop: Ref<boolean>
  platform: Ref<Platform>
  isMac: ComputedRef<boolean>
  /** 是否渲染自绘窗口控制按钮（桌面 + 非 macOS；macOS 走系统交通灯）。 */
  showControls: ComputedRef<boolean>
  onMinimize: () => void
  onMaximize: () => void
  onClose: () => void
} {
  const isDesktop = ref(isDesktopHost())
  const platform = ref<Platform>('linux')
  let settled = false
  async function settle(): Promise<void> {
    if (settled) return
    settled = true
    // Tauri 轨走 `window_platform`（编译期目标 OS）；浏览器轨 `navigator` 兜底。
    platform.value = await landesk.platform()
  }
  settle()

  const isMac = computed(() => platform.value === 'macos')
  // macOS 保留系统交通灯 → 不画自绘按钮；Windows/Linux 全自绘 → 画。浏览器预览不画（无窗口可控）。
  const showControls = computed(() => isDesktop.value && !isMac.value)

  return {
    isDesktop,
    platform,
    isMac,
    showControls,
    onMinimize: () => { void landesk.windowMinimize() },
    onMaximize: () => { void landesk.windowToggleMaximize() },
    onClose: () => { void landesk.windowClose() },
  }
}
