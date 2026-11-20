<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useUi, toast } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import type { DeviceRow, DeviceKind } from '../types/index'
import Icon from '../components/common/Icon.vue'
import EmptyState from '../components/common/EmptyState.vue'

const ui = useUi()
const { local, devices, deviceLoading, loadDevices, refreshTempPassword, loadLocal } = useLanDesk()

const quickCode = ref('41C98B7E')
const search = ref('')
const history = ['9D21 44AF', '77BE 0C13', 'DE53 2A90', 'B104 6F8D']

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return devices.value
  return devices.value.filter((d) =>
    d.name.toLowerCase().includes(q) || d.addr.includes(q) || d.id.toLowerCase().includes(q))
})
const onlineCount = computed(() => devices.value.filter((d) => d.online).length)

function connect(device: DeviceRow) {
  if (!device.online) { toast('设备离线，无法连接', 'warning'); return }
  ui.startSession(device, device.relay ? 'p2p' : 'inner')
}
function quickConnect() {
  const code = quickCode.value.replace(/\s/g, '')
  if (!code) { toast('请输入对方设备码', 'warning'); return }
  ui.startSession({ name: code, id: code, res: '1920×1080' }, 'p2p')
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
    <!-- 快速连接（通栏） -->
    <section class="ld-card ld-card__pad">
      <div class="ld-card__head">
        <span class="ld-quick__ico"><Icon name="zap" :size="18" /></span>
        <div class="ld-quick__heading">
          <span class="ld-card__title">快速连接</span>
          <span class="ld-card__sub">输入对方设备码即可发起远程控制</span>
        </div>
      </div>
      <div class="ld-quick__grid">
        <div class="ld-quick__input">
          <span class="ld-quick__prefix">#</span>
          <input v-model="quickCode" class="ll-input ll-input--mono ld-quick__code" placeholder="请输入对方 8 位设备码" maxlength="12" @keyup.enter="quickConnect" />
          <span class="ld-quick__hint">8 位设备码</span>
        </div>
        <button class="ll-btn ll-btn--primary ld-quick__btn" @click="quickConnect"><Icon name="link" :size="15" /> 远程控制</button>
        <button class="ll-btn ll-btn--default ld-quick__btn" @click="toast('本次会话使用临时密码', 'info')"><Icon name="key" :size="15" /> 一次性密码</button>
      </div>
      <div class="ld-recent">
        <span class="ld-recent__label">最近连接</span>
        <button v-for="h in history" :key="h" class="ld-chip" @click="quickCode = h.replace(/\s/g,'')">
          <Icon name="history" :size="12" /> {{ h }}
        </button>
      </div>
    </section>

    <!-- 下方两栏：左（本机设备码 + 运行信息）｜右（局域网设备） -->
    <div class="ld-device-columns">
      <!-- 左栏 -->
      <aside class="ld-local-col">
        <!-- 本机设备码卡（淡蓝底） -->
        <section class="ld-card ld-card__pad ld-local-card">
          <div class="ld-card__head">
            <span class="ld-local__ico"><Icon name="monitor" :size="19" /></span>
            <div>
              <div class="ld-card__title ld-local__name">
                {{ local.name }}
                <Icon name="edit" :size="13" class="ld-local__edit" />
              </div>
              <div class="ld-local__online"><span class="ll-dot" :class="local.online ? 'll-dot--success' : ''" /> {{ local.online ? '在线 · 可被连接' : '离线' }}</div>
            </div>
          </div>

          <div class="ld-field">
            <div class="ld-field__label">本机设备码</div>
            <div class="ld-field__box">
              <span class="ld-field__value ld-code ld-local__code">{{ local.deviceCode }}</span>
              <button class="ld-field__copy" title="复制设备码" @click="copy(local.deviceCode, '设备码已复制')"><Icon name="copy" :size="16" /></button>
            </div>
          </div>

          <div class="ld-field">
            <div class="ld-field__label">
              <span>临时密码</span>
              <button class="ld-field__refresh" @click="refreshTempPassword(); toast('临时密码已刷新')"><Icon name="refresh" :size="12" /> 刷新</button>
            </div>
            <div class="ld-field__box">
              <span class="ld-field__value ld-code ld-local__pw">{{ local.tempPassword }}</span>
              <button class="ld-field__copy" title="复制临时密码" @click="copy(local.tempPassword, '临时密码已复制')"><Icon name="copy" :size="16" /></button>
            </div>
          </div>

          <div class="ld-local__tip"><Icon name="shield" :size="14" /> 临时密码每 5 分钟刷新，仅存派生值，支持一次性密码连接</div>
        </section>

        <!-- 本机运行信息卡（白底，独立） -->
        <section class="ld-card ld-card__pad ld-info-card">
          <div class="ld-info-card__title">本机运行信息</div>
          <div class="ld-kvline"><span class="ld-kvline__k">mDNS 发现</span><span class="ld-kvline__v"><span class="ll-dot" :class="local.mdns ? 'll-dot--success' : 'll-dot--idle'" /> {{ local.mdns ? '已启用' : '未启用' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">信令服务</span><span class="ld-kvline__v ld-mono">{{ local.signaling || '未配置' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">中继服务</span><span class="ld-kvline__v"><span class="ll-dot" :class="local.relay ? 'll-dot--success' : 'll-dot--idle'" /> {{ local.relay ? '可用' : '不可用' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">编码能力</span><span class="ld-kvline__v"><span v-if="local.enc.length" class="ll-tag">{{ local.enc.join(' · ') }}</span><span v-else class="ld-form-help">未探测</span></span></div>
          <div class="ll-divider" />
          <div class="ld-sess-actions">
            <button class="ll-btn ll-btn--default" @click="ui.navigate('settings')"><Icon name="settings" :size="14" /> 受控设置</button>
            <button class="ll-btn ll-btn--default" @click="ui.navigate('logs')"><Icon name="file" :size="14" /> 运行日志</button>
          </div>
        </section>
      </aside>

      <!-- 右栏：局域网设备 -->
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
          <article v-for="d in filtered" :key="d.id" class="ld-dev-card" :class="{ 'is-offline': !d.online }" @click="connect(d)">
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
          <span class="ld-footbar__hint">点击设备卡任意位置即可快速连接</span>
        </div>
      </section>
    </div>
  </div>
</template>
