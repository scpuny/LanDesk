<script setup lang="ts">
// 根组件：桌面应用框架（顶栏 + 左轨 + 动态视图）+ 全局 Toast/确认弹窗/主题面板。
// 布局与主题架构移植自 lanLink；页面为 LanDesk 业务。
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { useUi } from './composables/useUi'
import { useTheme } from './composables/useTheme'
import Icon from './components/common/Icon.vue'
import LeftRail from './components/layout/LeftRail.vue'
import WindowControls from './components/layout/WindowControls.vue'
import ThemePanel from './components/dialogs/ThemePanel.vue'
import ConfirmDialog from './components/dialogs/ConfirmDialog.vue'
import DeviceList from './views/DeviceList.vue'
import Remote from './views/Remote.vue'
import Session from './views/Session.vue'
import Settings from './views/Settings.vue'
import Logs from './views/Logs.vue'
import { useWindowControls } from './composables/useWindowControls'

const { state, toasts, confirmRequest, resolveConfirm } = useUi()
const { pref } = useTheme()
// 自绘主窗体：macOS 保留系统交通灯 → 顶栏左侧留白（.ld-topbar--mac）；Win/Linux 全自绘三键。
const { isMac } = useWindowControls()

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
  remote: ['远程控制', '发起连接 · 本机设备码 · 运行状态'],
  devices: ['我的设备', '3 秒找到设备 · 一次点击连接'],
  session: ['远程会话', '实时画面 · 加密通道 · 状态可视'],
  settings: ['设置', '受控配置 · 连接方式 · 日志策略'],
  logs: ['日志中心', '环形缓冲 2000 条 · 按级别筛 · 可导出'],
}
const title = computed(() => titles[state.mainView]?.[0] || '')
const subtitle = computed(() => titles[state.mainView]?.[1] || '')

const views = { remote: Remote, devices: DeviceList, session: Session, settings: Settings, logs: Logs }
const currentView = computed(() => views[state.mainView] || Remote)
</script>

<template>
  <div class="ld-app">
    <!-- 会话页为独立沉浸窗口，不显示常规框架 -->
    <template v-if="state.mainView === 'session'">
      <component :is="currentView" :key="state.mainView" />
    </template>

    <template v-else>
      <!-- 自绘主窗体：去系统标题栏后，顶栏即窗口标题栏。
           静态区（logo/标题/占位）标 `data-tauri-drag-region` 可拖动窗口（Win/Linux 无边框窗）；
           可点击控件（主题/通知/头像/窗口三键）**不**标（Tauri drag.js 约定须排除交互元素）。
           macOS（交通灯 overlay）：左侧 `--mac` 留白避开原生交通灯，系统标题栏负责拖动。 -->
      <header class="ld-topbar" :class="{ 'ld-topbar--mac': isMac }" data-tauri-drag-region>
        <div class="ld-brand">
          <span class="ld-brand__logo"><Icon name="monitor" :size="20" :width="2" /></span>
          <span class="ld-brand__text">
            <span class="ld-brand__name">LanDesk</span>
            <span class="ld-brand__sub">REMOTE DESKTOP</span>
          </span>
        </div>
        <div class="ld-topbar__titles" data-tauri-drag-region>
          <span class="ld-topbar__title">{{ title }}</span>
          <span class="ld-topbar__sub">{{ subtitle }}</span>
        </div>
        <div class="ld-topbar__spacer" data-tauri-drag-region />
        <div class="ld-topbar__actions">
          <button class="ld-icon-btn ld-theme-toggle" :title="'主题：' + pref" @click="themeOpen = !themeOpen"><Icon :name="pref === 'dark' ? 'moon' : 'sun'" :size="17" /></button>
          <button class="ld-icon-btn" title="通知" @click="toasts.length && toasts[0]"><Icon name="bell" :size="17" /></button>
        </div>
        <!-- Win/Linux 自绘控制键；主框去掉「最大化」（本软件只需 最小化 + 关闭），macOS 走系统交通灯不渲染；浏览器预览隐藏 -->
        <WindowControls noMax />
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
