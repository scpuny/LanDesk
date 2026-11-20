<script setup lang="ts">
import { useUi } from '../../composables/useUi'
import { useLanDesk } from '../../composables/useLanDesk'
import type { MainView } from '../../types/index'
import Icon from '../common/Icon.vue'

const { state, navigate } = useUi()
const { devices, local } = useLanDesk()

const navs: { id: MainView; label: string; icon: string }[] = [
  { id: 'remote', label: '远程控制', icon: 'zap' },
  { id: 'devices', label: '我的设备', icon: 'monitor' },
  { id: 'session', label: '远程会话', icon: 'video' },
  { id: 'logs', label: '日志中心', icon: 'terminal' },
  { id: 'settings', label: '设置', icon: 'settings' },
]
const recent = [
  { name: '客厅-iMac', online: true },
  { name: '会议室大屏', online: true },
  { name: '财务工作站', online: false },
]
const onlineCount = () => devices.value.filter((d) => d.online).length
</script>

<template>
  <aside class="ld-rail">
    <div class="ld-nav-group">工作区</div>
    <button v-for="n in navs" :key="n.id" class="ld-nav" :class="{ 'is-active': state.mainView === n.id }" @click="navigate(n.id)">
      <Icon :name="n.icon" :size="17" />
      <span class="ld-nav__text">{{ n.label }}</span>
      <span v-if="n.id === 'devices'" class="ld-nav__count">{{ onlineCount() }}</span>
    </button>

    <div class="ld-nav-group">最近连接</div>
    <button v-for="r in recent" :key="r.name" class="ld-nav" @click="navigate('devices')">
      <span class="ll-dot" :class="r.online ? 'll-dot--success' : ''" />
      <span class="ld-nav__text">{{ r.name }}</span>
    </button>

    <div class="ld-rail__spacer" />
    <div class="ld-rail-card">
      <div class="ld-rail-card__row"><span class="ll-dot ll-dot--success" /><span>本机空闲可用</span></div>
      <div class="ld-rail-card__row"><span>连接路径</span><span class="ld-rail-card__v">内网直连</span></div>
      <div class="ld-rail-card__row"><span>设备码</span><span class="ld-rail-card__v ld-mono">{{ local.deviceCode }}</span></div>
    </div>
    <div class="ld-rail__foot">LanDesk v1.0.0 · 自托管</div>
  </aside>
</template>
