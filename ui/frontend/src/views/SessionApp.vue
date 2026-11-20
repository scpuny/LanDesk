<script setup lang="ts">
// 会话独立 OS 窗体（label "session"，decorations:false 自绘，完整 最小化/最大化/关闭）的根组件。
// 职责单一：装载时从壳取「发起请求」（target + path，`readSessionLaunch`），写入共享 `useUi` state
// 供 `Session.vue` 渲染；并订阅 `session://open` 事件——同一会话窗已开着时主窗再次发起（切设备），
// 就地重设目标。`Session.vue` 内的沉浸视图 + 完整三键 + 拖拽区不改动，直接复用。
// 关闭会话窗 = 点自绘 × 键（`window_close` 关调用方窗口即本会话窗；主窗仍在，App 不退出）。
import { onMounted, onBeforeUnmount } from 'vue'
import { useUi } from '../composables/useUi'
import { readSessionLaunch, watchSessionOpen } from '../composables/useSessionWindow'
import type { ConnMode } from '../types/index'
import Session from './Session.vue'

const { startSession } = useUi()
let unwatch: (() => void) | null = null

function asPath(p: string | null): ConnMode {
  return p === 'p2p' || p === 'relay' || p === 'inner' ? p : 'inner'
}

onMounted(async () => {
  // 首帧：壳里存了本次发起的目标（session_window_open 写入 session_launch）。
  const launch = await readSessionLaunch()
  if (launch.target) startSession(launch.target, asPath(launch.path))
  // 同一会话窗已开时主窗再发起（切设备）→ session://open 事件重设目标。
  unwatch = watchSessionOpen((p) => {
    if (p.target) startSession(p.target, asPath(p.path))
  })
})
onBeforeUnmount(() => { if (unwatch) unwatch() })
</script>

<template>
  <Session standalone />
</template>
