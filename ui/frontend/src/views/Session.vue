<script setup lang="ts">
// 会话页：状态机驱动（state.sessionState），对齐 V2 设计稿四态 + PRD 七态。
// 状态枚举做 class，中文文案放映射表，禁止中文写进 CSS 类名。
// P1.2：重连浮层改用后端退避计划（reconnectPlan），替代硬编码 2/5 与 1/2/4/8s。
import { computed, ref, onBeforeUnmount, onMounted, watch } from 'vue'
import { useUi } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import { useWindowControls } from '../composables/useWindowControls'
import { landesk, isDesktopHost } from '../lib/landesk'
import { watchSessionCloseRequest } from '../composables/useSessionWindow'
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

// 关闭会话窗（macOS 红绿灯 ×、Win/Linux 自绘 ×）统一走二次确认：
// 壳拦截原生 CloseRequested → 发 session://close-requested → 这里弹窗；确认后优雅断开并放行关窗。
// 内部流程（断开/返回/去设置）仍直接 `windowClose()`（壳标记放行），不重复弹窗。
let unwatchClose: (() => void) | null = null
let closeConfirming = false
async function confirmCloseWindow(): Promise<void> {
  if (closeConfirming) return
  closeConfirming = true
  try {
    const ok = await confirm({
      title: '关闭会话窗口',
      message: '关闭窗口将断开当前远程会话，确定要关闭吗？',
      confirmText: '关闭窗口',
      cancelText: '取消',
    })
    if (!ok) return
    if (!props.standalone) { closeSession(); return } // 浏览器单页预览：切回主界面
    if (isLive.value) {
      // 关闭优先：后端断开失败也继续放行，避免窗口关不掉。
      try { await closeSessionLive() } catch { /* ignore */ }
    }
    await landesk.sessionWindowCloseConfirmed()
  } finally {
    closeConfirming = false
  }
}
/** 拿当前目标 + 输入的访问密码真正建连（会话窗「建立连接」/鉴权失败「重新验证」共用）。
 *  密码可空：后端受控服务按密码门禁，空密码会明确失败（对方未开启受控服务 / 密码错），
 *  由后端错误区分，不在前端猜测。 */
async function connectFromTarget(): Promise<void> {
  if (!state.sessionTarget) { toast('没有连接目标', 'warning'); return }
  setState('auth')
  const r = await connectSession(state.sessionTarget.id, password.value)
  if (r.ok) { setState('running'); toast('会话已建立，正在接收画面') }
  else { setState('authfailed'); toast(r.error || '鉴权失败', 'error') }
}

async function verify() {
  if (!password.value) { toast('请输入临时密码', 'warning'); return }
  // live：凭临时密码重连当前目标（鉴权重试）；非 live 演示态直接转 running。
  if (isLive.value && state.sessionTarget) { await connectFromTarget(); return }
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
// 「键鼠」按钮：非连续输入，仅演示/手动启一次转发开关提示；真实逐事件转发见下方输入捕获。
function onToolKeyboard() {
  if (isLive.value) { toast('键鼠转发已启用（画面区移动/点击/滚动/键盘已接管）', 'info'); return }
  toast('键鼠控制已启用', 'info')
}
function onToolDisplay() {
  if (isLive.value) { liveSend({ type: 'displaySwitch', index: 0 }); return }
  toast('已切换到主显示器', 'info')
}

// ---------- #69 全屏：会话 OS 窗最大化/还原（与顶栏 □ 键同一命令）----------
// live(桌面 Tauri) 走 `window_toggle_maximize`；浏览器 / 非桌面宿主 no-op（保留演示 toast）。
const isFullscreen = ref(false)
async function onToolFullscreen() {
  if (isDesktopHost()) {
    const nowMax = await landesk.windowToggleMaximize()
    isFullscreen.value = nowMax
    return
  }
  toast('已进入全屏（预览）', 'info')
}

// ---------- #70 画质真缩放：自适应 / 原始尺寸 切 canvas object-fit ----------
// 自适应 = contain（letterbox 居中，不裁切）；原始尺寸 = none（1:1 内禀像素，溢出可滚）。
// （`scaleMode` 已在组件顶部声明；此处只补切换逻辑 + 计算 object-fit。）
function toggleScale() { scaleMode.value = scaleMode.value === '自适应' ? '原始尺寸' : '自适应' }
const scaleFit = computed(() => (scaleMode.value === '原始尺寸' ? 'none' : 'contain'))

// ---------- #71 截图：抓当前画面 canvas 帧 → 落盘（Tauri 写文件 / 浏览器下载）----------
async function onToolScreenshot() {
  const c = canvasEl.value
  if (!c || state.sessionState !== 'running') { toast('暂无画面可截', 'info'); return }
  let dataUrl = ''
  try { dataUrl = c.toDataURL('image/png') }
  catch { toast('截图失败（画面未就绪）', 'error'); return }
  const ok = await landesk.screenshotSave(dataUrl)
  toast(ok ? '截图已保存' : '截图保存失败', ok ? 'success' : 'error')
}

// ---------- #73 更多：下拉菜单（截图 / 画质切换 / 全屏 / 断开）----------
const moreOpen = ref(false)
function toggleMore() { moreOpen.value = !moreOpen.value }
function closeMore() { moreOpen.value = false }
// 点菜单/触发按钮以外关闭：监听 document，排除 `.ld-sess-more` 内部。
function onMoreDocPointerDown(e: PointerEvent): void {
  if (!moreOpen.value) return
  const t = e.target as HTMLElement | null
  if (t && t.closest('.ld-sess-more')) return
  moreOpen.value = false
}
onMounted(() => document.addEventListener('pointerdown', onMoreDocPointerDown))
onBeforeUnmount(() => document.removeEventListener('pointerdown', onMoreDocPointerDown))

// ---------- #72 键鼠转发：画面区捕获输入 → ControlAction → session_send_control ----------
// 仅 live 运行态启用（`inputEnabled`）；坐标归一化 0..65535（FR-06.3，对齐后端 MouseMove）。
// 修饰键位 MOD_CTRL=1/SHIFT=2/ALT=4/META=8（lp::keycodes）。
let mouseButtons = 0   // 已按下按钮位（1=左 2=右 4=中），mousemove 携带
function inputEnabled(): boolean { return isLive.value && state.sessionState === 'running' }
function modsOf(e: { ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }): number {
  let m = 0
  if (e.ctrlKey) m |= 1
  if (e.shiftKey) m |= 2
  if (e.altKey) m |= 4
  if (e.metaKey) m |= 8
  return m
}
/** client 坐标 → 归一化 0..65535（按 canvas object-fit 显示区换算，letterbox 区外返回 null）。 */
function canvasNorm(clientX: number, clientY: number): { x: number; y: number } | null {
  const c = canvasEl.value
  if (!c) return null
  const rect = c.getBoundingClientRect()
  const vw = c.width || 1280
  const vh = c.height || 720
  const ar = vw / vh
  const rectAr = rect.width / rect.height
  let dw: number, dh: number, dx: number, dy: number
  if (scaleMode.value === '原始尺寸') {
    // 原始尺寸：canvas CSS 盒子即视频区（1:1）
    dw = rect.width; dh = rect.height; dx = rect.left; dy = rect.top
  } else if (rectAr > ar) {
    dh = rect.height; dw = rect.height * ar; dx = rect.left + (rect.width - dw) / 2; dy = rect.top
  } else {
    dw = rect.width; dh = rect.width / ar; dx = rect.left; dy = rect.top + (rect.height - dh) / 2
  }
  const rx = (clientX - dx) / dw
  const ry = (clientY - dy) / dh
  if (rx < 0 || rx > 1 || ry < 0 || ry > 1) return null
  return { x: Math.round(rx * 65535), y: Math.round(ry * 65535) }
}
/** 静默发一帧（连续输入不用 toast，仅失败时轻提示一次）。 */
function sendQuiet(action: ControlAction): void {
  void sendControl(action).then((r) => {
    if (!r.ok && action.type !== 'mouseMove') toast(r.error || '控制帧发送失败', 'error')
  })
}
function onMouseMove(e: MouseEvent): void {
  if (!inputEnabled()) return
  const n = canvasNorm(e.clientX, e.clientY)
  if (!n) return
  sendQuiet({ type: 'mouseMove', x: n.x, y: n.y, buttons: mouseButtons, mods: modsOf(e) })
}
function onMouseDown(e: MouseEvent): void {
  if (!inputEnabled()) return
  canvasEl.value?.focus()   // 焦点到画面 canvas → 键盘事件（#72）被捕获
  const bit = e.button === 0 ? 1 : e.button === 2 ? 2 : e.button === 1 ? 4 : 0
  if (bit) mouseButtons |= bit
  const n = canvasNorm(e.clientX, e.clientY)
  if (n) sendQuiet({ type: 'mouseMove', x: n.x, y: n.y, buttons: mouseButtons, mods: modsOf(e) })
  if (bit) sendQuiet({ type: 'mouseButton', button: e.button as 0 | 1 | 2, down: 1 })
}
function onMouseUp(e: MouseEvent): void {
  if (!inputEnabled()) return
  const bit = e.button === 0 ? 1 : e.button === 2 ? 2 : e.button === 1 ? 4 : 0
  if (bit) mouseButtons &= ~bit
  if (bit) sendQuiet({ type: 'mouseButton', button: e.button as 0 | 1 | 2, down: 0 })
}
function onWheel(e: WheelEvent): void {
  if (!inputEnabled()) return
  // 归一：~1 单位/格（后端 i16 dy 为滚轮增量，负=下）
  const dy = Math.round(e.deltaY / 40)
  const dx = Math.round(e.deltaX / 40)
  if (dx !== 0 || dy !== 0) sendQuiet({ type: 'mouseWheel', dx, dy })
}
/** JS `event.code`（布局无关）→ Lkc u32 键码表（对齐 crates/lp/src/keycodes.rs，0x10-0x6D）。 */
const LKC_BY_CODE: Record<string, number> = {
  KeyA: 0x10, KeyB: 0x11, KeyC: 0x12, KeyD: 0x13, KeyE: 0x14, KeyF: 0x15, KeyG: 0x16, KeyH: 0x17, KeyI: 0x18,
  KeyJ: 0x19, KeyK: 0x1A, KeyL: 0x1B, KeyM: 0x1C, KeyN: 0x1D, KeyO: 0x1E, KeyP: 0x1F, KeyQ: 0x20, KeyR: 0x21,
  KeyS: 0x22, KeyT: 0x23, KeyU: 0x24, KeyV: 0x25, KeyW: 0x26, KeyX: 0x27, KeyY: 0x28, KeyZ: 0x29,
  Digit0: 0x30, Digit1: 0x31, Digit2: 0x32, Digit3: 0x33, Digit4: 0x34, Digit5: 0x35, Digit6: 0x36, Digit7: 0x37, Digit8: 0x38, Digit9: 0x39,
  Numpad0: 0x30, Numpad1: 0x31, Numpad2: 0x32, Numpad3: 0x33, Numpad4: 0x34, Numpad5: 0x35, Numpad6: 0x36, Numpad7: 0x37, Numpad8: 0x38, Numpad9: 0x39,
  Semicolon: 0x40, Equal: 0x41, Comma: 0x42, Minus: 0x43, Period: 0x44, Slash: 0x45, BracketLeft: 0x46, BracketRight: 0x47, Quote: 0x48, Backslash: 0x49,
  Enter: 0x50, Escape: 0x51, Backspace: 0x52, Tab: 0x53, Space: 0x54,
  ArrowUp: 0x58, ArrowDown: 0x59, ArrowLeft: 0x5A, ArrowRight: 0x5B,
  PageUp: 0x5C, PageDown: 0x5D, Home: 0x5E, End: 0x5F, Insert: 0x60, Delete: 0x61,
  F1: 0x62, F2: 0x63, F3: 0x64, F4: 0x65, F5: 0x66, F6: 0x67, F7: 0x68, F8: 0x69, F9: 0x6A, F10: 0x6B, F11: 0x6C, F12: 0x6D,
}
function onKeyDown(e: KeyboardEvent): void {
  if (!inputEnabled()) return
  const k = LKC_BY_CODE[e.code]
  if (k === undefined) return
  sendQuiet({ type: 'keyDown', key: k, mods: modsOf(e) })
  // 会话窗接管键盘，阻止本地副作用（F11 全屏 / Tab 焦点 / 复制粘贴等）。
  if (['Space', 'Tab', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'F11'].includes(e.code)) e.preventDefault()
}
function onKeyUp(e: KeyboardEvent): void {
  if (!inputEnabled()) return
  const k = LKC_BY_CODE[e.code]
  if (k === undefined) return
  sendQuiet({ type: 'keyUp', key: k, mods: modsOf(e) })
}
function onKeyBlur(): void {
  // 失焦时所有键抬起（避免「粘键」），仅 live 运行态。
  if (!inputEnabled()) return
  // 无具体键信息 → 统一抬起 4 个修饰键（Lkc 单键值 0x01 Ctrl / 0x02 Shift / 0x03 Alt / 0x04 Meta）。
  for (const k of [0x01, 0x02, 0x03, 0x04]) sendQuiet({ type: 'keyUp', key: k, mods: 0 })
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
      codec: 'avc1.42c01f',      // H.264 baseline（openh264 真编码 / 占位后端均走此档）
      // 不指定 codedWidth/codedHeight：受控端真采屏分辨率随显示器变化，
      // 让解码器从码流 SPS 推断实际尺寸，避免配置与真实分辨率不符而解码失败。
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
  countFrame(data.length)   // #74：累计画面字节 → 窗口吞吐
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

// ---------- #74 底部实时指标：会话时长 / 帧率 / 吞吐（客户端可算，不依赖新后端字段）----------
// 时长：进 running 记 `runningSince`，1s 计时器刷新；离开 running 冻结并清 timer。
// 帧率：每收到一画面帧 `frameCount++`，按 1s 窗口算 fps（~30fps 稳态）。
// 吞吐：来帧字节数累计 / 窗口（PlaceholderEncoder 画面 NALU 字节近似码率；live 真实码率仍用状态快照 bitrate）。
let runningSince = 0        // 进 running 时刻（ms）
let elapsed = ref(0)        // 已运行时长（秒，展示 00:MM:SS）
let fpsNow = 0              // 当前帧率（窗口值）
let throughputKb = 0        // 当前吞吐（KB/s，画面字节近似）
let metricTimer: ReturnType<typeof setInterval> | null = null
let frameWin = 0           // 本窗口帧数
let byteWin = 0            // 本窗口画面字节数
let winStart = 0          // 窗口起点（ms）

function startMetrics(): void {
  if (metricTimer) return
  runningSince = performance.now()
  frameWin = 0; byteWin = 0; winStart = performance.now()
  metricTimer = setInterval(() => {
    const now = performance.now()
    const span = now - winStart
    if (span >= 1000) {
      const secs = span / 1000
      fpsNow = Math.round((frameWin / secs) * 10) / 10
      throughputKb = Math.round((byteWin / 1024) / secs)
      frameWin = 0; byteWin = 0; winStart = now
    }
    elapsed.value = Math.round((now - runningSince) / 1000)
  }, 1000)
}
function stopMetrics(): void {
  if (metricTimer) { clearInterval(metricTimer); metricTimer = null }
  elapsed.value = 0; fpsNow = 0; throughputKb = 0
}
/** 供 `onVideoFrame` 每帧累加（在解码器 output 回调里调）。 */
function countFrame(bytes: number): void {
  frameWin++; byteWin += bytes
}
/** 底部展示时长 00:MM:SS（>99h 钳 99:59:59）。 */
const elapsedLabel = computed(() => {
  const s = elapsed.value
  const h = Math.min(99, Math.floor(s / 3600))
  const m = Math.floor((s % 3600) / 60)
  const ss = s % 60
  const p = (n: number) => String(n).padStart(2, '0')
  return `${p(h)}:${p(m)}:${p(ss)}`
})

// live 运行态起画面流，离开运行态（断开 / 鉴权失败 / 关闭）停；幂等。同时起/停实时指标计时。
watch(() => state.sessionState, (s) => {
  if (s === 'running' && isLive.value) {
    videoStop = startVideoStream(onVideoFrame)
    startMetrics()
  } else {
    stopVideo()
    stopMetrics()
  }
})

// live：后端会话状态快照 → 本地 `state`（状态机 + 目标 + 连接方式 + 度量），驱动顶部标签 + 底部状态栏。
// 非 live 时 `liveSession` 恒 null，watcher 不写 `state`（演示路径保留）。
//
// ★ 竞态防护：会话独立窗体装载时，`SessionApp.onMounted` 先 `readSessionLaunch()` 拿目标并
// `startSession(target)` 把本地置为 auth+target；本组件 `onMounted` 又 `loadSessionStatus()`
// 拉后端快照。此刻后端会话尚未建立（`status_view()` = idle + target:null）。两个 async hook
// 完成顺序不确定——若 idle 快照后到，会把刚设好的 auth+target 覆盖回 idle → 用户看到
// 「尚未连接远程设备」白屏（点击设备没反应）。故 idle 且无 target 的快照一律忽略：它代表
// 「控制端本地还没有活跃会话」，不应覆盖前端待连接目标 / 鉴权失败等本地态。
watch(liveSession, (snap) => {
  if (!snap) return
  if (snap.stateCode === 'idle' && !snap.target) return
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
  // 原生关闭请求（macOS 交通灯 ×）先弹确认，不直接关窗。
  unwatchClose = watchSessionCloseRequest(() => { void confirmCloseWindow() })
  // live：拉当前会话状态快照（未连 = idle，据此渲染顶部标签 + 底部状态栏）。
  await loadSessionStatus()
})
onBeforeUnmount(() => {
  if (unwatchClose) unwatchClose()
  if (timer) clearInterval(timer)
  stopVideo(); stopMetrics()
})
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
        <button class="ld-sess-tool" @click="toggleScale"><Icon name="minimize" :size="14" /> {{ scaleMode }} <Icon name="chevronDown" :size="12" /></button>
        <button class="ld-sess-tool" @click="onToolFullscreen"><Icon name="fullscreen" :size="14" /> {{ isFullscreen ? '还原' : '全屏' }}</button>
        <div class="ld-sess-more">
          <button class="ld-sess-tool" @click="toggleMore"><Icon name="dots" :size="14" /> 更多</button>
          <div v-if="moreOpen" class="ld-sess-more__menu" @click="closeMore">
            <button @click="onToolScreenshot"><Icon name="camera" :size="14" /> 截图</button>
            <button @click="toggleScale"><Icon name="minimize" :size="14" /> {{ scaleMode === '自适应' ? '切原始尺寸' : '切自适应' }}</button>
            <button @click="onToolFullscreen"><Icon name="fullscreen" :size="14" /> {{ isFullscreen ? '还原' : '全屏' }}</button>
            <button @click="disconnect"><Icon name="power" :size="14" /> 断开连接</button>
          </div>
        </div>
        <button v-if="state.sessionState === 'running' || state.sessionState === 'reconnecting'" class="ll-btn ll-btn--danger ll-btn--sm" @click="disconnect">
          <Icon name="power" :size="13" /> 断开连接
        </button>
        <button v-else-if="state.sessionState === 'authfailed'" class="ll-btn ll-btn--default ll-btn--sm" @click="closeSession">× 关闭会话</button>
      </div>
      <!-- Win/Linux 自绘三键（min/max/close）；macOS 走系统交通灯不渲染；浏览器预览隐藏 -->
      <WindowControls confirm-close @close="confirmCloseWindow" />
    </header>

    <!-- 画面区 -->
    <div class="ld-sess-canvas">
      <!-- 运行态：铺满画面 + 悬浮工具栏 + 降级提示 -->
      <template v-if="state.sessionState === 'running'">
        <div class="ld-sess-canvas__bg" style="background:radial-gradient(ellipse at 60% 40%, #3a5a7a 0%, #1d3040 45%, #0a1018 100%)" />
        <!-- P2② 视频数据面：live 且运行态时 WebCodecs 解码画面渲染到此 canvas（复用 __bg 定位，铺满）；
             非 live（演示预览）不渲染，保留渐变底。canvas 内禀 1280×720，CSS object-fit 由「画质」切换（contain↔none）。
             live 运行态：画面区捕获键鼠/滚轮/键盘（#72）逐事件转发后端 session_send_control。 -->
        <canvas
          v-if="isLive && liveSession?.stateCode === 'running'"
          ref="canvasEl"
          class="ld-sess-canvas__bg"
          width="1280"
          height="720"
          :style="{ background:'#000', zIndex:1, objectFit: scaleFit, cursor:'crosshair' }"
          tabindex="0"
          @mousemove="onMouseMove"
          @mousedown="onMouseDown"
          @mouseup="onMouseUp"
          @wheel.prevent="onWheel"
          @keydown="onKeyDown"
          @keyup="onKeyUp"
          @blur="onKeyBlur"
        />
        <div style="position:absolute;top:14px;left:16px;font-size:12px;color:#e8edf5;opacity:.8">● 远程桌面 · 自适应画质 · 60fps</div>
        <div style="position:absolute;top:14px;right:16px;font-size:12px;color:#e8edf5;opacity:.8;display:flex;align-items:center;gap:6px"><Icon name="shield" :size="13" /> QUIC + AES-256-GCM 加密通话</div>
        <div class="ld-sess-warnbar" v-if="autoHideHint">
          <Icon name="alertTriangle" :size="14" /> 硬件编码不可用，已切换 OpenH264，会话保持不中断 · 重置 ×1
          <button @click="autoHideHint = false"><Icon name="close" :size="13" /></button>
        </div>
        <div class="ld-sess-toolrail">
          <button title="剪贴板同步" @click="onToolClipboard"><Icon name="clipboard" :size="17" /></button>
          <button title="键鼠（画面区已接管输入）" @click="onToolKeyboard"><Icon name="keyboard" :size="17" /></button>
          <button title="截图（抓当前画面帧）" @click="onToolScreenshot"><Icon name="camera" :size="17" /></button>
          <button title="显示器切换" @click="onToolDisplay"><Icon name="monitor" :size="17" /></button>
          <button title="全屏" @click="onToolFullscreen"><Icon name="fullscreen" :size="17" /></button>
          <button class="is-danger" title="断开" @click="disconnect"><Icon name="power" :size="17" /></button>
        </div>
        <div style="position:absolute;bottom:12px;left:50%;transform:translateX(-50%);font-size:11px;color:#e8edf5;opacity:.5">鼠标移上 3 秒自动隐藏工具栏</div>
      </template>

      <!-- 鉴权中（已输密码、后端正在建连） -->
      <div v-else-if="state.sessionState === 'auth'" class="ld-sess-overlay">
        <div class="ld-sess-spinner" />
        <div class="ld-sess-title" style="font-size:18px">正在建立安全会话</div>
        <div class="ld-sess-desc">设备码 {{ targetName }} · 正在协商密钥与建立加密通道…</div>
        <div class="ld-sess-steps">
          <span class="ld-sess-step is-active"><Icon name="activity" :size="14" /> 协商密钥 · X25519</span>
          <span class="ld-sess-step"><Icon name="clock" :size="14" /> 探测连接路径 · 内网直连</span>
          <span class="ld-sess-step"><Icon name="clock" :size="14" /> 建立加密通道 · 等待中</span>
        </div>
        <button class="ll-btn ll-btn--default" @click="cancel"><Icon name="close" :size="14" /> 取消连接</button>
      </div>

      <!-- 待鉴权：拿到目标后先输入对方访问密码（对齐「点设备→输密码→看桌面」闭环） -->
      <div v-else-if="state.sessionState === 'idle' && state.sessionTarget" class="ld-sess-overlay">
        <div class="ld-sess-bigicon"><Icon name="key" :size="30" /></div>
        <div class="ld-sess-title" style="font-size:18px">连接 {{ targetName }}</div>
        <div class="ld-sess-desc">请输入对方设备的访问密码（对方设置页「访问密码」，留空则对方未开启受控服务）</div>
        <div class="ld-sess-password-card">
          <div style="display:flex;align-items:center;gap:8px">
            <Icon name="key" :size="16" style="color:var(--ll-color-brand)" />
            <b>对方访问密码</b>
          </div>
          <input v-model="password" type="password" class="ll-input" placeholder="请输入对方设备访问密码"
                 @keyup.enter="connectFromTarget" />
          <div class="ld-sess-actions">
            <button class="ll-btn ll-btn--primary" @click="connectFromTarget"><Icon name="link" :size="14" /> 建立连接</button>
            <button class="ll-btn ll-btn--default" @click="cancel"><Icon name="close" :size="14" /> 取消</button>
          </div>
        </div>
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
      <span class="ld-metric" title="会话时长 · 画面吞吐 · 帧率（live 实时，非 live 为 0）">{{ elapsedLabel }}　↑ {{ throughputKb }}KB/s　{{ fpsNow }}fps</span>
    </footer>
  </div>
</template>
