// 诊断包生成（运行时问题可定位）。
// 纯函数：把环境快照 + 全部日志组装成可读文本，不依赖 DOM / 宿主，便于单测。
// 下载函数单独抽出，浏览器与 Tauri WebView 均走 Blob + <a download>。
import type { RawLogLine } from './landesk'
import type { LanDeskConfig, LocalInfo, ControlledStatus } from '../types/index'

/** 版本号——由 vite.config.ts 从 tauri.conf.json 注入（发布打包时为 tag 版本）。 */
declare const __APP_VERSION__: string

export interface DiagnosticsInput {
  version?: string
  platform: string
  userAgent: string
  generatedAt: string
  local: LocalInfo | null
  controlled: ControlledStatus | null
  config: LanDeskConfig | null
  logs: RawLogLine[]
  /** 非 live（浏览器预览）态标注，避免把 mock 数据当真机证据。 */
  preview?: boolean
}

function appVersion(): string {
  try { return typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : 'unknown' }
  catch { return 'unknown' }
}

function kv(label: string, value: unknown): string {
  if (value === null || value === undefined || value === '') return `${label}: —`
  return `${label}: ${String(value)}`
}

/** 从日志里挑出与运行时问题相关的关键词行，作为报告顶部"可疑证据"。 */
const SUSPECT_KEYS = ['degrad', 'no_permission', 'blank_frame', 'permission', 'firewall', 'timeout', 'error', 'fail']
function suspectLines(logs: RawLogLine[]): RawLogLine[] {
  return logs.filter((l) => {
    const hay = `${l.mod} ${l.msg} ${l.kv ?? ''}`.toLowerCase()
    return SUSPECT_KEYS.some((k) => hay.includes(k))
  }).slice(-30)
}

function fmtTime(t: number): string {
  if (!t) return '--:--:--.---'
  const ms = Math.floor(t / 1000)
  const micro = Math.floor(t % 1000)
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${String(micro).padStart(3, '0')}`
}

/** 组装可读诊断报告文本。敏感信息（访问码/临时码）只标注"是否设置"，不明文导出。 */
export function buildDiagnosticReport(input: DiagnosticsInput): string {
  const out: string[] = []
  const ver = input.version ?? appVersion()
  out.push('===== LanDesk 诊断报告 =====')
  out.push(kv('生成时间', input.generatedAt))
  out.push(kv('应用版本', ver))
  out.push(kv('平台', input.platform))
  out.push(kv('数据来源', input.preview ? '浏览器预览（mock，非真机）' : '真机运行时（live）'))
  out.push(kv('User-Agent', input.userAgent))
  out.push('')

  out.push('---- 本机 ----')
  out.push(kv('设备名', input.local?.name))
  out.push(kv('设备码', input.local?.deviceCode))
  out.push(kv('在线', input.local === null ? '—' : (input.local.online ? '是' : '否')))
  out.push(kv('mDNS', input.local === null ? '—' : (input.local.mdns ? '开启' : '关闭')))
  out.push(kv('信令', input.local?.signaling))
  out.push(kv('中继', input.local === null ? '—' : (input.local.relay ? '开启' : '关闭')))
  out.push(kv('编码器', input.local?.enc?.join(', ')))
  out.push('')

  out.push('---- 受控服务 ----')
  out.push(kv('服务中', input.controlled === null ? '—' : (input.controlled.serving ? '是' : '否')))
  out.push(kv('监听端口', input.controlled?.port))
  out.push(kv('设备码', input.controlled?.deviceCode))
  out.push(kv('临时码', input.controlled === null ? '—' : (input.controlled.tempCodeEnabled ? '已启用' : '未启用')))
  out.push('')

  out.push('---- 配置摘要（密码脱敏） ----')
  out.push(kv('访问密码', input.config ? (input.config.password ? '已设置' : '未设置') : '—'))
  out.push(kv('码率档', input.config?.preset))
  out.push(kv('脏矩形', input.config === null ? '—' : (input.config.dirty_rect ? '开' : '关')))
  out.push('')

  const suspects = suspectLines(input.logs)
  out.push(`---- 可疑事件（最近 ${suspects.length} 条）----`)
  if (suspects.length === 0) out.push('（无明显降级/失败/超时关键词）')
  for (const l of suspects) {
    out.push(`[${fmtTime(l.t)}] ${l.level} ${l.mod} ${l.msg} ${l.kv ?? ''}`.trimEnd())
  }
  out.push('')

  out.push(`---- 全部日志（${input.logs.length} 条）----`)
  for (const l of input.logs) {
    out.push(`[${fmtTime(l.t)}] ${l.level} ${l.mod} ${l.msg} ${l.kv ?? ''}`.trimEnd())
  }
  out.push('')
  out.push('===== 报告结束 =====')
  return out.join('\n')
}

/** 触发下载（浏览器 + Tauri WebView 通用）。文件名形如 landsdk-diagnostic-20261123-212700.txt。 */
export function downloadReport(text: string, filename: string): void {
  const blob = new Blob([text], { type: 'text/plain;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  URL.revokeObjectURL(url)
}

/** 生成带时间戳的文件名。 */
export function diagnosticFilename(now = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0')
  const ts = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`
  return `landsdk-diagnostic-${ts}.txt`
}
