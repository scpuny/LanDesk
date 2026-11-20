<script setup lang="ts">
// 会话页：状态机驱动（state.sessionState），对齐 V2 设计稿四态 + PRD 七态。
// 状态枚举做 class，中文文案放映射表，禁止中文写进 CSS 类名。
// P1.2：重连浮层改用后端退避计划（reconnectPlan），替代硬编码 2/5 与 1/2/4/8s。
import { computed, ref, onBeforeUnmount, onMounted } from 'vue'
import { useUi } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import type { SessionState, ConnMode } from '../types/index'
import Icon from '../components/common/Icon.vue'

const { state, navigate, toast, confirm } = useUi()
const { reconnectPlan, loadReconnectPlan } = useLanDesk()

// 退避计划（后端下发；非 live 用默认预览序列 1/2/4/8/16s · 最多 5 次）。
const maxAttempts = computed(() => reconnectPlan.value.max_attempts)
const waitLabel = computed(() => reconnectPlan.value.waits.map((w) => `${w}s`).join(' / '))
const nextWait = computed(() => {
  const waits = reconnectPlan.value.waits
  const i = Math.min(retryAttempt.value, waits.length) - 1
  return waits[i] ?? waits[waits.length - 1] ?? 0
})

const states: Record<SessionState, { label: string; dot: string }> = {
  idle: { label: '未连接', dot: '' },
  auth: { label: '鉴权中', dot: 'll-dot--brand' },
  running: { label: '运行中', dot: 'll-dot--success ll-dot--pulse' },
  reconnecting: { label: '重连中', dot: 'll-dot--warning' },
  authfailed: { label: '鉴权失败', dot: 'll-dot--danger' },
  nopath: { label: '无可用路径', dot: 'll-dot--danger' },
  closed: { label: '已关闭', dot: '' },
}
const stateInfo = computed(() => states[state.sessionState] || states.idle)
const targetName = computed(() => state.sessionTarget?.name || state.sessionTarget?.id || '远程设备')
// 顶部状态标签：重连态动态拼接「重连中 N/max」（N 为演示计数，max 来自退避计划）。
const stateLabel = computed(() => stateInfo.value.label +
  (state.sessionState === 'reconnecting' ? ` ${Math.min(retryAttempt.value, maxAttempts.value)}/${maxAttempts.value}` : ''))

const scaleMode = ref('自适应')
const retryAttempt = ref(2)
const password = ref('')
const autoHideHint = ref(true)

const pathTagMap: Record<ConnMode, string> = { inner: '内网直连', p2p: 'P2P 直连', relay: '中继转发' }
const pathClassMap: Record<ConnMode, string> = { inner: 'll-tag--brand', p2p: 'll-tag--success', relay: 'll-tag--warning' }
const pathTag = computed(() => pathTagMap[state.path] || '内网直连')
const pathClass = computed(() => pathClassMap[state.path])

function setState(s: SessionState) { state.sessionState = s }
function closeSession() { setState('closed'); navigate('devices') }
function cancel() { setState('idle'); navigate('devices'); toast('已取消连接', 'info') }
async function disconnect() {
  const ok = await confirm({ title: '断开连接', message: '确定要断开当前远程会话吗？', confirmText: '断开', cancelText: '保持连接' })
  if (ok) { setState('closed'); navigate('devices'); toast('会话已断开', 'info') }
}
function verify() {
  if (!password.value) { toast('请输入临时密码', 'warning'); return }
  setState('running'); toast('鉴权成功，会话已建立')
}
function retryNow() { setState('running'); toast('已手动重试并连接成功') }

// 演示用：允许点击状态步骤切换（浏览器预览体验四态）
const demoStates: SessionState[] = ['auth', 'running', 'reconnecting', 'authfailed']
function cycleState() {
  const i = demoStates.indexOf(state.sessionState)
  setState(demoStates[(i + 1) % demoStates.length])
}

let timer = null
onMounted(() => { loadReconnectPlan() })
onBeforeUnmount(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="ld-session">
    <!-- 顶部工具条 -->
    <header class="ld-sess-top">
      <span class="ld-sess-state"><span class="ll-dot" :class="stateInfo.dot" /> {{ stateLabel }}</span>
      <span class="ld-sess-target">
        <Icon name="monitor" :size="14" /> {{ targetName }}
        <span class="ll-tag" :class="pathClass">{{ pathTag }}</span>
        <span v-if="state.sessionTarget?.res" class="ld-mono">{{ state.sessionTarget.res }}</span>
      </span>
      <!-- 演示态切换器（仅开发预览用） -->
      <button class="ld-sess-tool" style="opacity:.6" title="切换会话状态预览" @click="cycleState"><Icon name="sliders" :size="14" /> 状态预览</button>
      <div class="ld-sess-top__spacer" />
      <div class="ld-sess-tools">
        <button class="ld-sess-tool" @click="scaleMode = scaleMode === '自适应' ? '原始尺寸' : '自适应'"><Icon name="minimize" :size="14" /> {{ scaleMode }} <Icon name="chevronDown" :size="12" /></button>
        <button class="ld-sess-tool" @click="toast('已进入全屏（预览）', 'info')"><Icon name="fullscreen" :size="14" /> 全屏</button>
        <button class="ld-sess-tool" @click="toast('更多操作', 'info')"><Icon name="dots" :size="14" /> 更多</button>
        <button v-if="state.sessionState === 'running' || state.sessionState === 'reconnecting'" class="ll-btn ll-btn--danger ll-btn--sm" @click="disconnect">
          <Icon name="power" :size="13" /> 断开连接
        </button>
        <button v-else-if="state.sessionState === 'authfailed'" class="ll-btn ll-btn--default ll-btn--sm" @click="closeSession">× 关闭会话</button>
      </div>
    </header>

    <!-- 画面区 -->
    <div class="ld-sess-canvas">
      <!-- 运行态：铺满画面 + 悬浮工具栏 + 降级提示 -->
      <template v-if="state.sessionState === 'running'">
        <div class="ld-sess-canvas__bg" style="background:radial-gradient(ellipse at 60% 40%, #3a5a7a 0%, #1d3040 45%, #0a1018 100%)" />
        <div style="position:absolute;top:14px;left:16px;font-size:12px;color:#e8edf5;opacity:.8">● 远程桌面 · 自适应画质 · 60fps</div>
        <div style="position:absolute;top:14px;right:16px;font-size:12px;color:#e8edf5;opacity:.8;display:flex;align-items:center;gap:6px"><Icon name="shield" :size="13" /> QUIC + AES-256-GCM 加密通话</div>
        <div class="ld-sess-warnbar" v-if="autoHideHint">
          <Icon name="alertTriangle" :size="14" /> 硬件编码不可用，已切换 OpenH264，会话保持不中断 · 重置 ×1
          <button @click="autoHideHint = false"><Icon name="close" :size="13" /></button>
        </div>
        <div class="ld-sess-toolrail">
          <button title="剪贴板同步" @click="toast('剪贴板同步已开启', 'info')"><Icon name="clipboard" :size="17" /></button>
          <button title="键鼠" @click="toast('键鼠控制已启用', 'info')"><Icon name="keyboard" :size="17" /></button>
          <button title="截图" @click="toast('已截图', 'success')"><Icon name="camera" :size="17" /></button>
          <button title="显示器切换" @click="toast('已切换到主显示器', 'info')"><Icon name="monitor" :size="17" /></button>
          <button title="全屏" @click="toast('全屏（预览）', 'info')"><Icon name="fullscreen" :size="17" /></button>
          <button class="is-danger" title="断开" @click="disconnect"><Icon name="power" :size="17" /></button>
        </div>
        <div style="position:absolute;bottom:12px;left:50%;transform:translateX(-50%);font-size:11px;color:#e8edf5;opacity:.5">鼠标移上 3 秒自动隐藏工具栏</div>
      </template>

      <!-- 建立中 -->
      <div v-else-if="state.sessionState === 'auth'" class="ld-sess-overlay">
        <div class="ld-sess-spinner" />
        <div class="ld-sess-title" style="font-size:18px">正在建立安全会话</div>
        <div class="ld-sess-desc">设备码 {{ targetName }} · 已耗时 2.4 秒</div>
        <div class="ld-sess-steps">
          <span class="ld-sess-step is-done"><Icon name="check" :size="14" /> 协商密钥 · X25519 完成</span>
          <span class="ld-sess-step is-active"><Icon name="activity" :size="14" /> 探测连接路径 · ICE 打洞中</span>
          <span class="ld-sess-step"><Icon name="clock" :size="14" /> 建立加密通道 · 等待中</span>
        </div>
        <button class="ll-btn ll-btn--default" @click="cancel"><Icon name="close" :size="14" /> 取消连接</button>
      </div>

      <!-- 重连中 -->
      <div v-else-if="state.sessionState === 'reconnecting'" class="ld-sess-overlay">
        <div class="ld-sess-bigicon is-warn"><Icon name="refresh" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">重连中 ({{ Math.min(retryAttempt, maxAttempts) }}/{{ maxAttempts }})…正在尝试 P2P 直连</div>
        <div class="ld-sess-desc">QUIC 连接已中断，采用指数退避重连（{{ waitLabel }}）</div>
        <div class="ld-sess-progress"><i :style="{ width: (Math.min(retryAttempt, maxAttempts) / maxAttempts * 100) + '%' }" /></div>
        <div style="display:flex;gap:40px;font-size:12px;color:var(--ll-session-text-dim)">
          <span>第 {{ retryAttempt }} 次尝试 · 下次重试 {{ nextWait }} 秒后</span><span>{{ Math.min(retryAttempt, maxAttempts) }} / {{ maxAttempts }}</span>
        </div>
        <div class="ld-sess-actions">
          <button class="ll-btn ll-btn--primary" @click="retryNow"><Icon name="refresh" :size="14" /> 立即重试</button>
          <button class="ll-btn ll-btn--default" @click="disconnect"><Icon name="power" :size="14" /> 断开连接</button>
        </div>
        <div style="font-size:11px;color:var(--ll-session-text-dim);opacity:.7;margin-top:8px">画面保留最后一帧 · 重连成功后强制 IDR 刷新</div>
      </div>

      <!-- 鉴权失败 -->
      <div v-else-if="state.sessionState === 'authfailed'" class="ld-sess-overlay">
        <div class="ld-sess-bigicon is-danger"><Icon name="lock" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">鉴权失败，无法建立会话</div>
        <div class="ld-sess-desc">原因：临时密码错误（已连续失败 2 次），请核对对方设备屏幕上显示的临时密码后重试</div>
        <div class="ld-sess-password-card">
          <div style="display:flex;align-items:center;gap:8px">
            <Icon name="key" :size="16" style="color:var(--ll-color-brand)" />
            <b>重新输入临时密码</b>
            <span class="ll-tag ll-tag--danger" style="margin-left:auto">剩余 3 次机会</span>
          </div>
          <input v-model="password" type="password" class="ll-input ll-input--error" placeholder="请输入临时密码" />
          <div class="ld-form-error">密码不正确，请向设备所有者索取最新临时密码</div>
          <div class="ld-sess-actions">
            <button class="ll-btn ll-btn--primary" @click="verify"><Icon name="shield" :size="14" /> 重新验证</button>
            <button class="ll-btn ll-btn--default" @click="toast('已发送一次性密码请求', 'info')"><Icon name="zap" :size="14" /> 一次性密码</button>
          </div>
        </div>
        <div style="display:flex;gap:16px;font-size:12px;color:var(--ll-color-brand);margin-top:4px">
          <span class="ll-link" @click="cancel">← 返回设备列表</span>
          <span class="ll-link" @click="navigate('logs')">▤ 查看失败详情日志</span>
        </div>
      </div>

      <!-- 无可用路径 -->
      <div v-else-if="state.sessionState === 'nopath'" class="ld-sess-overlay">
        <div class="ld-sess-bigicon is-danger"><Icon name="wifioff" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">无可用路径</div>
        <div class="ld-sess-desc">P2P 打洞失败，且未启用中继。请在设置中开启中继兜底，或确认双方网络可达。</div>
        <div class="ld-sess-actions">
          <button class="ll-btn ll-btn--primary" @click="navigate('settings')"><Icon name="settings" :size="14" /> 去开启中继</button>
          <button class="ll-btn ll-btn--default" @click="cancel">返回设备列表</button>
        </div>
      </div>

      <!-- 空闲 / 已关闭 -->
      <div v-else class="ld-sess-overlay">
        <div class="ld-sess-bigicon" style="background:rgba(255,255,255,.08);color:#9aa6ba"><Icon name="monitor" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">{{ state.sessionState === 'closed' ? '会话已关闭' : '尚未连接远程设备' }}</div>
        <div class="ld-sess-desc">在设备列表选择一台设备，或在首页输入对方设备码发起连接</div>
        <button class="ll-btn ll-btn--primary" @click="navigate('devices')"><Icon name="monitor" :size="14" /> 去选择设备</button>
      </div>
    </div>

    <!-- 底部状态栏（7 指标可视化） -->
    <footer class="ld-sess-bottom">
      <span class="ld-metric"><span class="ll-dot" :class="stateInfo.dot" /> {{ stateInfo.label }}</span>
      <span class="ld-metric">{{ pathTag }}</span>
      <span class="ld-metric"><Icon name="chip" :size="12" /> <b>{{ state.metrics.encoder }}</b> H.264</span>
      <span class="ld-metric"><span class="ld-signal" :class="state.sessionState === 'running' ? 'dot-good' : ''"><i/><i/><i/><i/></span> <b>{{ state.metrics.latency }}ms</b></span>
      <span class="ld-metric">丢包 <b>{{ state.metrics.loss }}%</b></span>
      <span class="ld-metric">码率 <b>{{ state.metrics.bitrate }}Mbps</b></span>
      <span class="ld-metric">重置 <b>×{{ state.metrics.reset }}</b></span>
      <span class="ld-sess-bottom__spacer" />
      <span class="ld-metric">00:12:47　↑ 38.2MB　60fps</span>
    </footer>
  </div>
</template>
