<script setup lang="ts">
// 我的设备页（选择侧）：局域网设备列表。点卡片上的「连接」按钮发起连接 →
// `launchSession`（Tauri 开独立会话 OS 窗 / 浏览器开单页沉浸 session）。
// 卡片本体不可点（避免误触弹会话窗），只有显式「连接」按钮发起。
// 发起侧（快速连接 + 本机卡）已拆到 Remote 页。
import { ref, computed, onMounted } from 'vue'
import { toast } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import { launchSession } from '../composables/useSessionWindow'
import type { DeviceRow, DeviceKind } from '../types/index'
import Icon from '../components/common/Icon.vue'
import EmptyState from '../components/common/EmptyState.vue'

const { devices, deviceLoading, loadDevices, loadLocal } = useLanDesk()

const search = ref('')

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return devices.value
  return devices.value.filter((d) =>
    d.name.toLowerCase().includes(q) || d.addr.includes(q) || d.id.toLowerCase().includes(q))
})
const onlineCount = computed(() => devices.value.filter((d) => d.online).length)

function connect(device: DeviceRow) {
  if (!device.online) { toast('设备离线，无法连接', 'warning'); return }
  launchSession(device, device.relay ? 'p2p' : 'inner')
}
function copy(text: string, label = '已复制') {
  navigator.clipboard?.writeText(text).then(() => toast(label)).catch(() => toast(label))
}
function kindIcon(kind: DeviceKind): string {
  const map: Record<DeviceKind, string> = { monitor: 'monitor', laptop: 'laptop', chip: 'chip', server: 'server', phone: 'phone' }
  return map[kind] || 'monitor'
}
function lastSeen(n: number): string { return n ? `离线 ${n} 分钟` : '刚刚在线' }

onMounted(() => { loadDevices(); loadLocal() })
</script>

<template>
  <div class="ld-content">
    <!-- 局域网设备（选择侧） -->
    <section class="ld-dev-section">
      <div class="ld-dev-head">
        <h2 class="ld-h2">局域网设备</h2>
        <span class="ll-tag ll-tag--success"><span class="ll-dot ll-dot--success" /> {{ onlineCount }} 台在线</span>
        <div class="ld-dev-head__tools">
          <input v-model="search" class="ll-input ld-dev-head__search" placeholder="搜索设备名称或地址" />
          <button class="ll-btn ll-btn--default" @click="loadDevices(); toast('已重新扫描')"><Icon name="refresh" :size="14" /> 刷新</button>
        </div>
      </div>

      <div v-if="deviceLoading" class="ld-dev-grid">
        <div v-for="i in 6" :key="i" class="ld-card ld-card__pad" style="height:150px">
          <div class="ll-skeleton" style="height:34px;width:60%;margin-bottom:16px" />
          <div class="ll-skeleton" style="height:18px;width:80%;margin-bottom:12px" />
          <div class="ll-skeleton" style="height:36px;width:100%" />
        </div>
      </div>
      <div v-else-if="filtered.length === 0" class="ld-card ld-card__pad">
        <EmptyState description="未发现局域网设备" size="md">
          <template #footer>
            <p class="ld-form-help" style="text-align:center;margin:8px 0 12px">确认在同一网段且已开启 mDNS</p>
            <button class="ll-btn ll-btn--default" @click="loadDevices()"><Icon name="refresh" :size="14" /> 重新扫描</button>
          </template>
        </EmptyState>
      </div>
      <div v-else class="ld-dev-grid">
        <!-- 卡片本体不可点（避免误触弹会话窗）；连接只从卡片内显式「连接」按钮发起 -->
        <article v-for="d in filtered" :key="d.id" class="ld-dev-card" :class="{ 'is-offline': !d.online }">
          <div class="ld-dev-card__top">
            <span class="ld-dev-card__icon"><Icon :name="kindIcon(d.kind)" :size="20" /></span>
            <div class="ld-dev-card__id">
              <div class="ld-dev-card__name ld-truncate">{{ d.name }}</div>
              <div class="ld-dev-card__meta">{{ d.addr }} · {{ d.res }}</div>
            </div>
            <span class="ld-dev-card__status">
              <span class="ld-status-pill"><span class="ll-dot" :class="d.online ? 'll-dot--success' : ''" /> {{ d.online ? '在线' : '离线' }}</span>
            </span>
          </div>
          <div class="ld-dev-card__code">
            <span class="ld-code">{{ d.id }}</span>
            <button class="ld-dev-card__copy" title="复制设备码" @click.stop="copy(d.id, '设备码已复制')"><Icon name="copy" :size="14" /></button>
          </div>
          <div class="ld-dev-card__tags">
            <span v-for="e in d.enc" :key="e" class="ll-tag">{{ e }}</span>
            <span v-if="d.relay && d.online" class="ll-tag">中继可用</span>
            <span v-if="!d.online" class="ll-tag ll-tag--plain">{{ lastSeen(d.lastSeen) }}</span>
          </div>
          <div class="ld-dev-card__foot">
            <button class="ll-btn" :class="d.online ? 'll-btn--primary' : 'll-btn--default'" :disabled="!d.online" @click.stop="connect(d)">
              <Icon name="link" :size="14" /> {{ d.online ? '连接' : '不可用' }}
            </button>
            <button class="ll-btn ll-btn--default ld-dev-card__more" title="更多" @click.stop="toast('设备操作：详情 / 重命名 / 删除', 'info')"><Icon name="dots" :size="16" /></button>
          </div>
        </article>
      </div>

      <div class="ld-card ld-card__pad ld-footbar">
        <span>局域网发现 {{ devices.length }} 台</span>
        <span>在线可连 {{ onlineCount }} 台</span>
        <span>上次扫描 刚刚</span>
        <span class="ld-footbar__hint">点击卡片上的「连接」按钮发起会话（卡片本体不可点）</span>
      </div>
    </section>
  </div>
</template>
