<script setup lang="ts">
// 会话页：状态机驱动（state.sessionState），对齐 V2 设计稿四态 + PRD 七态。
// 状态枚举做 class，中文文案放映射表，禁止中文写进 CSS 类名。
// P1.2：重连浮层改用后端退避计划（reconnectPlan），替代硬编码 2/5 与 1/2/4/8s。
import { computed, ref, onBeforeUnmount, onMounted, watch } from 'vue'
import { useUi } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import { useWindowControls } from '../composables/useWindowControls'
import { landesk } from '../lib/landesk'
import type { SessionState, ConnMode, ControlAction, VideoFrameMsg } from '../types/index'
import Icon from '../components/common/Icon.vue'
import WindowControls from '../components/layout/WindowControls.vue'

// `standalone`：会话独立 OS 窗体（label "session"，Tauri `SessionApp` 渲染）时为 true——
// 「关闭会话 / 返回设备列表 / 去开启中继」等回退动作**关闭本会话窗**（`window_close` 关调用方窗口），
// 而非切主框 mainView（主框另有自己的设备/设置/日志视图）。浏览器单页沉浸（App.vue 直挂）为 false，走 navigate。
const props = defineProps<{ standalone?: boolean }>()

const { state, navigate, toast, confirm } = useUi()
// 自绘主窗体：会话页顶栏即窗口标题栏（全屏沉浸态）。Win/Linux 无边框 → 静态区可拖 + 右侧自绘三键；
// macOS overlay 交通灯 → 左侧留白（.ld-sess-top--mac）。
const { isMac } = useWindowControls()
const {
  reconnectPlan, loadReconnectPlan,
  isLive, liveSession, loadSessionStatus, connectSession, sendControl, closeSessionLive,
  startVideoStream, stopVideoStream,
} = useLanDesk()

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
// 会话窗 standalone 时「返回/关闭」= 关本 OS 窗（主框仍在）；单页沉浸时切回主框设备页。
function goBack() {
  if (props.standalone) { void landesk.windowClose(); return }
  navigate('devices')
}
// 跳主框其它视图（设置/日志/设备）：standalone 会话窗没有这些视图 → 关窗回主框；单页沉浸直接 navigate。
function leave(view: 'devices' | 'settings' | 'logs') {
  if (props.standalone) { void landesk.windowClose(); return }
  navigate(view)
}
function closeSession() { setState('closed'); goBack() }
function cancel() { setState('idle'); goBack(); toast('已取消连接', 'info') }
async function disconnect() {
  const ok = await confirm({ title: '断开连接', message: '确定要断开当前远程会话吗？', confirmText: '断开', cancelText: '保持连接' })
  if (!ok) return
  // live：走后端 `session_close`（优雅断开 QUIC + 状态机落 Closed）；非 live 仅本地演示。
  if (isLive.value) await closeSessionLive()
  setState('closed'); goBack(); toast('会话已断开', 'info')
}
async function verify() {
  if (!password.value) { toast('请输入临时密码', 'warning'); return }
  // live：凭临时密码重连当前目标（鉴权重试）；非 live 演示态直接转 running。
  if (isLive.value && state.sessionTarget) {
    const r = await connectSession(state.sessionTarget.id, password.value)
    if (r.ok) { setState('running'); toast('鉴权成功，会话已建立') }
    else { setState('authfailed'); toast(r.error || '鉴权失败', 'error') }
    return
  }
  setState('running'); toast('鉴权成功，会话已建立')
}
async function retryNow() {
  // live：重新拨号当前目标（退避后手动重试）；非 live 演示态。
  if (isLive.value && state.sessionTarget) {
    const r = await connectSession(state.sessionTarget.id, password.value || '')
    if (r.ok) { setState('running'); toast('已手动重试并连接成功') }
    else { setState('reconnecting'); toast(r.error || '重连失败，继续退避', 'warning') }
    return
  }
  setState('running'); toast('已手动重试并连接成功')
}

// 演示用：允许点击状态步骤切换（仅非 live 浏览器预览体验四态；live 态由会话命令面驱动，禁用演示切换）。
const demoStates: SessionState[] = ['auth', 'running', 'reconnecting', 'authfailed']
function cycleState() {
  if (isLive.value) { toast('已连接真实会话，状态由后端驱动', 'info'); return }
  const i = demoStates.indexOf(state.sessionState)
  setState(demoStates[(i + 1) % demoStates.length])
}

// live：工具栏键鼠 / 剪贴板 / 显示器控制意图 → 后端 `session_send_control`（非 live 保留演示 toast）。
async function liveSend(action: ControlAction) {
  if (!isLive.value) return
  const r = await sendControl(action)
  if (!r.ok) toast(r.error || '控制帧发送失败', 'error')
  else if (action.type !== 'clipboardText' && action.type !== 'close') {
    // 键鼠/编码器/显示器/心跳等即时操作给轻提示（剪贴板/关闭无逐条 toast）。
    toast('已下发', 'success', 1200)
  }
}
// 工具栏各控制按钮（live 走后端 session_send_control，非 live 保留演示 toast）。
function onToolClipboard() {
  if (isLive.value) { liveSend({ type: 'clipboardText', text: '' }); return }
  toast('剪贴板同步已开启', 'info')
}
function onToolKeyboard() {
  if (isLive.value) { liveSend({ type: 'keyDown', key: 0x10, mods: 0 }); return }
  toast('键鼠控制已启用', 'info')
}
function onToolDisplay() {
  if (isLive.value) { liveSend({ type: 'displaySwitch', index: 0 }); return }
  toast('已切换到主显示器', 'info')
}

// ---------- 视频数据面（P2 遗留 ②）：live 运行态 WebCodecs 解码 → canvas 渲染 ----------
// 画面帧通路（与 useLanDesk.startVideoStream 配套）：受控端 capture→encode→`VideoDgram` 分片
// →QUIC datagram→控制端 `JitterBuffer` 重组→宿主逐帧送达（Tauri poll / 浏览器 SSE）→
// WebCodecs `VideoDecoder`（H.264）解码→`<canvas>` 渲染。非 live 不启用（演示态走渐变底）。
const canvasEl = ref<HTMLCanvasElement | null>(null)
let videoStop: (() => void) | null = null   // 画面流 stop 句柄（useLanDesk 提供）
let decoder: VideoDecoder | null = null     // WebCodecs 解码器（惰性配置）
let decoderReady = false                     // 当前 codec 会话是否已 configure
let baseTs = 0                               // 帧时间戳基线（ms，~30fps 递增）

/** base64 → Uint8Array（`EncodedVideoChunk.data` 需 `ArrayBufferView`）。 */
function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

/** 惰性配置 H.264 `VideoDecoder`（首帧时按 canvas 内禀尺寸）。
 *  输出回调把解码帧画到 canvas 并 `close()` 释放帧内存；不支持 / 配置失败回 `false`。 */
function ensureDecoder(): boolean {
  if (decoderReady && decoder) return true
  const canvas = canvasEl.value
  if (!canvas || typeof VideoDecoder === 'undefined') return false
  const w = canvas.width || 1280
  const h = canvas.height || 720
  try {
    decoder = new VideoDecoder({
      output: (frame: VideoFrame) => {
        // 重新读 live ref，避免画到已卸载的旧 canvas 元素。
        const c = canvasEl.value
        if (c) {
          const ctx = c.getContext('2d')
          if (ctx) ctx.drawImage(frame, 0, 0, c.width || w, c.height || h)
        }
        frame.close()   // 已 draw 完，归还 WebCodecs 帧内存
      },
      error: (e: DOMException) => {
        stopVideoDecoder()   // 解码器错误：复位，等下一 IDR 起帧
        console.warn('[LanDesk] VideoDecoder error:', e.message)
      },
    })
    decoder.configure({
      codec: 'avc1.42c01f',      // H.264 baseline（PlaceholderEncoder 产出的裸 NALU 走此档）
      codedWidth: w,
      codedHeight: h,
      optimizeForLatency: true,
    })
    decoderReady = true
    baseTs = 0
    return true
  } catch {
    stopVideoDecoder()
    return false
  }
}

/** 收一帧（SSE / poll 回调）：base64 → `EncodedVideoChunk` → `decode`。
 *  仅 live 运行态处理；IDR 关键帧以 `type:'key'` 喂入（WebCodecs 据此重建解码上下文）。 */
function onVideoFrame(f: VideoFrameMsg): void {
  if (!isLive.value || state.sessionState !== 'running') return
  if (!ensureDecoder()) return
  if (!decoder || decoder.state === 'closed') return
  const data = b64ToBytes(f.nalu)
  const chunk = new EncodedVideoChunk({
    type: f.isIdr ? 'key' : 'delta',
    timestamp: baseTs,
    data: data.buffer as ArrayBuffer,
  })
  baseTs += 33   // ~30fps
  decoder.decode(chunk)
}

/** 复位 + 关闭解码器（会话离开运行态 / 卸载时调，幂等）。 */
function stopVideoDecoder(): void {
  if (decoder) {
    try { decoder.flush() } catch { /* ignore */ }
    try { decoder.close() } catch { /* ignore */ }
    decoder = null
  }
  decoderReady = false
  baseTs = 0
}

/** 停画面流（停传输 + 关解码器）。 */
function stopVideo(): void {
  if (videoStop) { videoStop(); videoStop = null }
  stopVideoDecoder()
}

// live 运行态起画面流，离开运行态（断开 / 鉴权失败 / 关闭）停；幂等。
watch(() => state.sessionState, (s) => {
  if (s === 'running' && isLive.value) videoStop = startVideoStream(onVideoFrame)
  else stopVideo()
})

// live：后端会话状态快照 → 本地 `state`（状态机 + 目标 + 连接方式 + 度量），驱动顶部标签 + 底部状态栏。
// 非 live 时 `liveSession` 恒 null，watcher 不写 `state`（演示路径保留）。
watch(liveSession, (snap) => {
  if (!snap) return
  state.sessionState = snap.stateCode
  state.path = snap.connModeCode
  state.sessionTarget = snap.target
    ? { name: snap.target.name, id: snap.target.id, res: snap.target.res }
    : null
  state.metrics = {
    encoder: snap.encoder || 'QSV',
    latency: snap.latencyMs ?? state.metrics.latency,
    loss: snap.lossPct ?? 0,
    bitrate: snap.bitrateMbps ?? state.metrics.bitrate,
    reset: snap.reset,
  }
}, { immediate: true })

let timer = null
onMounted(async () => {
  loadReconnectPlan()
  // live：拉当前会话状态快照（未连 = idle，据此渲染顶部标签 + 底部状态栏）。
  await loadSessionStatus()
})
onBeforeUnmount(() => { if (timer) clearInterval(timer); stopVideo() })
</script>

<template>
  <div class="ld-session">
    <!-- 顶部工具条（自绘主窗体下即窗口标题栏：Win/Linux 静态区可拖 + 右侧自绘三键；macOS 交通灯左侧留白） -->
    <header class="ld-sess-top" :class="{ 'ld-sess-top--mac': isMac }" data-tauri-drag-region>
      <span class="ld-sess-state" data-tauri-drag-region><span class="ll-dot" :class="stateInfo.dot" /> {{ stateLabel }}</span>
      <span class="ld-sess-target" data-tauri-drag-region>
        <Icon name="monitor" :size="14" /> {{ targetName }}
        <span class="ll-tag" :class="pathClass">{{ pathTag }}</span>
        <span v-if="state.sessionTarget?.res" class="ld-mono">{{ state.sessionTarget.res }}</span>
      </span>
      <!-- 演示态切换器（仅非 live 开发预览用；live 态由会话命令面驱动） -->
      <button v-if="!isLive" class="ld-sess-tool" style="opacity:.6" title="切换会话状态预览" @click="cycleState"><Icon name="sliders" :size="14" /> 状态预览</button>
      <div class="ld-sess-top__spacer" data-tauri-drag-region />
      <div class="ld-sess-tools">
        <button class="ld-sess-tool" @click="scaleMode = scaleMode === '自适应' ? '原始尺寸' : '自适应'"><Icon name="minimize" :size="14" /> {{ scaleMode }} <Icon name="chevronDown" :size="12" /></button>
        <button class="ld-sess-tool" @click="toast('已进入全屏（预览）', 'info')"><Icon name="fullscreen" :size="14" /> 全屏</button>
        <button class="ld-sess-tool" @click="toast('更多操作', 'info')"><Icon name="dots" :size="14" /> 更多</button>
        <button v-if="state.sessionState === 'running' || state.sessionState === 'reconnecting'" class="ll-btn ll-btn--danger ll-btn--sm" @click="disconnect">
          <Icon name="power" :size="13" /> 断开连接
        </button>
        <button v-else-if="state.sessionState === 'authfailed'" class="ll-btn ll-btn--default ll-btn--sm" @click="closeSession">× 关闭会话</button>
      </div>
      <!-- Win/Linux 自绘三键（min/max/close）；macOS 走系统交通灯不渲染；浏览器预览隐藏 -->
      <WindowControls />
    </header>

    <!-- 画面区 -->
    <div class="ld-sess-canvas">
      <!-- 运行态：铺满画面 + 悬浮工具栏 + 降级提示 -->
      <template v-if="state.sessionState === 'running'">
        <div class="ld-sess-canvas__bg" style="background:radial-gradient(ellipse at 60% 40%, #3a5a7a 0%, #1d3040 45%, #0a1018 100%)" />
        <!-- P2② 视频数据面：live 且运行态时 WebCodecs 解码画面渲染到此 canvas（复用 __bg 定位，铺满）；
             非 live（演示预览）不渲染，保留渐变底。canvas 内禀 1280×720，CSS object-fit: contain 自适应。 -->
        <canvas
          v-if="isLive && liveSession?.stateCode === 'running'"
          ref="canvasEl"
          class="ld-sess-canvas__bg"
          width="1280"
          height="720"
          style="background:#000;z-index:1"
        />
        <div style="position:absolute;top:14px;left:16px;font-size:12px;color:#e8edf5;opacity:.8">● 远程桌面 · 自适应画质 · 60fps</div>
        <div style="position:absolute;top:14px;right:16px;font-size:12px;color:#e8edf5;opacity:.8;display:flex;align-items:center;gap:6px"><Icon name="shield" :size="13" /> QUIC + AES-256-GCM 加密通话</div>
        <div class="ld-sess-warnbar" v-if="autoHideHint">
          <Icon name="alertTriangle" :size="14" /> 硬件编码不可用，已切换 OpenH264，会话保持不中断 · 重置 ×1
          <button @click="autoHideHint = false"><Icon name="close" :size="13" /></button>
        </div>
        <div class="ld-sess-toolrail">
          <button title="剪贴板同步" @click="onToolClipboard"><Icon name="clipboard" :size="17" /></button>
          <button title="键鼠" @click="onToolKeyboard"><Icon name="keyboard" :size="17" /></button>
          <button title="截图" @click="toast(isLive ? '截图帧已下发' : '已截图', 'success')"><Icon name="camera" :size="17" /></button>
          <button title="显示器切换" @click="onToolDisplay"><Icon name="monitor" :size="17" /></button>
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
          <span class="ll-link" @click="leave('logs')">▤ 查看失败详情日志</span>
        </div>
      </div>

      <!-- 无可用路径 -->
      <div v-else-if="state.sessionState === 'nopath'" class="ld-sess-overlay">
        <div class="ld-sess-bigicon is-danger"><Icon name="wifioff" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">无可用路径</div>
        <div class="ld-sess-desc">P2P 打洞失败，且未启用中继。请在设置中开启中继兜底，或确认双方网络可达。</div>
        <div class="ld-sess-actions">
          <button class="ll-btn ll-btn--primary" @click="leave('settings')"><Icon name="settings" :size="14" /> 去开启中继</button>
          <button class="ll-btn ll-btn--default" @click="cancel">返回设备列表</button>
        </div>
      </div>

      <!-- 空闲 / 已关闭 -->
      <div v-else class="ld-sess-overlay">
        <div class="ld-sess-bigicon" style="background:rgba(255,255,255,.08);color:#9aa6ba"><Icon name="monitor" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">{{ state.sessionState === 'closed' ? '会话已关闭' : '尚未连接远程设备' }}</div>
        <div class="ld-sess-desc">在设备列表选择一台设备，或在首页输入对方设备码发起连接</div>
        <button class="ll-btn ll-btn--primary" @click="leave('devices')"><Icon name="monitor" :size="14" /> 去选择设备</button>
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
