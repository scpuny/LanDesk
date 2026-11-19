<script setup lang="ts">
// 根组件：桌面应用框架（顶栏 + 左轨 + 动态视图）+ 全局 Toast/确认弹窗/主题面板。
// 布局与主题架构移植自 lanLink；页面为 LanDesk 业务。
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useUi } from './composables/useUi'
import { useTheme } from './composables/useTheme'
import Icon from './components/common/Icon.vue'
import LeftRail from './components/layout/LeftRail.vue'
import ThemePanel from './components/dialogs/ThemePanel.vue'
import ConfirmDialog from './components/dialogs/ConfirmDialog.vue'
import DeviceList from './views/DeviceList.vue'
import Session from './views/Session.vue'
import Settings from './views/Settings.vue'
import Logs from './views/Logs.vue'

const { state, toasts, confirmRequest, resolveConfirm } = useUi()
const { pref } = useTheme()

const themeOpen = ref(false)

// 点击面板外部关闭：监听 document，排除触发按钮与面板内部
function onDocPointerDown(e: PointerEvent): void {
  if (!themeOpen.value) return
  const target = e.target as HTMLElement | null
  if (!target) return
  if (target.closest('.ld-theme-pop')) return          // 面板内部
  if (target.closest('.ld-theme-toggle')) return        // 触发按钮自身（由它自己 toggle）
  themeOpen.value = false
}
onMounted(() => document.addEventListener('pointerdown', onDocPointerDown))
onBeforeUnmount(() => document.removeEventListener('pointerdown', onDocPointerDown))

const titles = {
  devices: ['我的设备', '3 秒找到设备 · 一次点击连接'],
  session: ['远程会话', '实时画面 · 加密通道 · 状态可视'],
  settings: ['设置', '受控配置 · 连接方式 · 日志策略'],
  logs: ['日志中心', '环形缓冲 2000 条 · 按级别筛 · 可导出'],
}
const title = computed(() => titles[state.mainView]?.[0] || '')
const subtitle = computed(() => titles[state.mainView]?.[1] || '')

const views = { devices: DeviceList, session: Session, settings: Settings, logs: Logs }
const currentView = computed(() => views[state.mainView] || DeviceList)
</script>

<template>
  <div class="ld-app">
    <!-- 会话页为独立沉浸窗口，不显示常规框架 -->
    <template v-if="state.mainView === 'session'">
      <component :is="currentView" :key="state.mainView" />
    </template>

    <template v-else>
      <header class="ld-topbar">
        <div class="ld-brand">
          <span class="ld-brand__logo"><Icon name="monitor" :size="20" :width="2" /></span>
          <span class="ld-brand__text">
            <span class="ld-brand__name">LanDesk</span>
            <span class="ld-brand__sub">REMOTE DESKTOP</span>
          </span>
        </div>
        <div class="ld-topbar__titles">
          <span class="ld-topbar__title">{{ title }}</span>
          <span class="ld-topbar__sub">{{ subtitle }}</span>
        </div>
        <div class="ld-topbar__spacer" />
        <div class="ld-topbar__actions">
          <button class="ld-icon-btn ld-theme-toggle" :title="'主题：' + pref" @click="themeOpen = !themeOpen"><Icon :name="pref === 'dark' ? 'moon' : 'sun'" :size="17" /></button>
          <button class="ld-icon-btn" title="通知" @click="toasts.length && toasts[0]"><Icon name="bell" :size="17" /></button>
          <span class="ld-avatar">陈</span>
        </div>
        <ThemePanel :open="themeOpen" />
      </header>

      <div class="ld-body">
        <LeftRail />
        <main class="ld-main">
          <component :is="currentView" :key="state.mainView" />
        </main>
      </div>
    </template>

    <!-- 全局 Toast -->
    <div class="ld-toasts">
      <div v-for="t in toasts" :key="t.id" class="ld-toast" :class="`is-${t.tone}`">
        <span class="ld-toast__ico"><Icon :name="t.tone === 'success' ? 'checkCircle' : t.tone === 'error' ? 'alertCircle' : 'info'" :size="16" /></span>
        {{ t.message }}
      </div>
    </div>

    <!-- 二次确认弹窗 -->
    <ConfirmDialog v-if="confirmRequest" @close="resolveConfirm(false)" />
  </div>
</template>
