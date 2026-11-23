<script setup lang="ts">
// 会话独立 OS 窗体（label "session"；macOS 原生交通灯 / Win/Linux 自绘三键）的根组件。
// 装载时从壳取「发起请求」（target + path，`readSessionLaunch`）写入共享 `useUi` state 供
// `Session.vue` 渲染；订阅 `session://open` 切目标。独立窗不挂 App.vue，故在此补挂
// 确认弹窗 + Toast——否则 `confirm()` 的 Promise 永远不会 resolve（关闭确认会卡住）。
import { onMounted, onBeforeUnmount } from 'vue'
import { useUi } from '../composables/useUi'
import { readSessionLaunch, watchSessionOpen, loadRecentTargets, recentTargets } from '../composables/useSessionWindow'
import type { ConnMode, SessionTarget } from '../types/index'
import Session from './Session.vue'
import ConfirmDialog from '../components/dialogs/ConfirmDialog.vue'
import Icon from '../components/common/Icon.vue'

const { startSession, toasts, confirmRequest, resolveConfirm } = useUi()
let unwatch: (() => void) | null = null

function asPath(p: string | null): ConnMode {
  return p === 'p2p' || p === 'relay' || p === 'inner' ? p : 'inner'
}

/** 重开兜底：壳里 launch.target 丢失时，用最近连接最新一条回填，避免显示「选择设备」白屏。
 *  recentTargets 由主窗 launchSession 持久化；会话独立窗不挂 App.vue，需在此显式 load。 */
function fallbackTarget(): SessionTarget | null {
  loadRecentTargets()
  const r = recentTargets.value[0]
  return r ? { id: r.code, name: r.name } : null
}

onMounted(async () => {
  // 首帧：壳里存了本次发起的目标（session_window_open 写入 session_launch）。
  const launch = await readSessionLaunch()
  // 真实桌面会话窗：停在「待鉴权」页让用户输入对方访问密码后再建连（非演示动画）。
  // 关窗重开后若后端 target 丢失 → 用最近连接兜底，回到上次设备而非「选择设备」。
  const target = launch.target ?? fallbackTarget()
  if (target) startSession(target, asPath(launch.path), 'await-auth')
  // 同一会话窗已开时主窗再发起（切设备）→ session://open 事件重设目标。
  unwatch = watchSessionOpen((p) => {
    if (p.target) startSession(p.target, asPath(p.path), 'await-auth')
  })
})
onBeforeUnmount(() => { if (unwatch) unwatch() })
</script>

<template>
  <Session standalone />
  <!-- 独立窗没有 App.vue：确认弹窗 / Toast 必须在此挂载，否则 confirm() 永不 resolve -->
  <div class="ld-toasts">
    <div v-for="t in toasts" :key="t.id" class="ld-toast" :class="`is-${t.tone}`">
      <span class="ld-toast__ico"><Icon :name="t.tone === 'success' ? 'checkCircle' : t.tone === 'error' ? 'alertCircle' : 'info'" :size="16" /></span>
      {{ t.message }}
    </div>
  </div>
  <ConfirmDialog v-if="confirmRequest" @close="resolveConfirm(false)" />
</template>
