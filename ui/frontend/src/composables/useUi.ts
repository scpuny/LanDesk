// 全局 UI 状态单例：主视图切换、会话状态、Toast、二次确认。
// 类型声明完整覆盖 state / toast / confirm。
import { reactive, ref, type Ref } from 'vue'
import type {
  MainView, SessionState, ConnMode, SessionMetrics, SessionTarget,
  ToastItem, ToastTone, ConfirmRequest,
} from '../types/index'

interface UiState {
  mainView: MainView
  sessionState: SessionState
  sessionTarget: SessionTarget | null
  path: ConnMode
  metrics: SessionMetrics
}

const state = reactive<UiState>({
  mainView: 'remote',
  sessionState: 'idle',
  sessionTarget: null,
  path: 'inner',
  metrics: { encoder: 'QSV', latency: 18, loss: 0, bitrate: 4.2, reset: 0 },
})

function navigate(view: MainView): void {
  state.mainView = view
}

function startSession(target: SessionTarget | null, path: ConnMode = 'inner'): void {
  state.sessionTarget = target
  state.path = path
  state.sessionState = 'auth'
  state.mainView = 'session'
}

// ---- Toast ----
const toasts: Ref<ToastItem[]> = ref<ToastItem[]>([])
let toastSeq = 0
export function toast(message: string, tone: ToastTone = 'success', duration = 2800): void {
  const id = ++toastSeq
  toasts.value.push({ id, message, tone })
  setTimeout(() => {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }, duration)
}

// ---- 二次确认（Promise 化）----
const confirmRequest: Ref<ConfirmRequest | null> = ref<ConfirmRequest | null>(null)
let confirmResolver: ((ok: boolean) => void) | null = null
export function confirm(req: {
  title: string
  message: string
  tone?: 'danger' | 'warning'
  confirmText?: string
  cancelText?: string
}): Promise<boolean> {
  confirmRequest.value = {
    title: req.title,
    message: req.message,
    tone: req.tone ?? 'danger',
    confirmText: req.confirmText ?? '确认',
    cancelText: req.cancelText ?? '取消',
  }
  return new Promise<boolean>((resolve) => { confirmResolver = resolve })
}
function resolveConfirm(ok: boolean): void {
  confirmRequest.value = null
  if (confirmResolver) { confirmResolver(ok); confirmResolver = null }
}

export interface UseUiReturn {
  state: UiState
  navigate: (view: MainView) => void
  startSession: (target: SessionTarget | null, path?: ConnMode) => void
  toasts: Ref<ToastItem[]>
  toast: typeof toast
  confirmRequest: Ref<ConfirmRequest | null>
  confirm: typeof confirm
  resolveConfirm: (ok: boolean) => void
}

export function useUi(): UseUiReturn {
  return {
    state, navigate, startSession,
    toasts, toast,
    confirmRequest, confirm, resolveConfirm,
  }
}
