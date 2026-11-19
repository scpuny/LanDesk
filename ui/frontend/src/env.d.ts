/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<Record<string, unknown>, Record<string, unknown>, any>
  export default component
}

/** 浏览器预览态注入的宿主标记（ui-server / Tauri 可置位以启用真实数据） */
interface Window {
  __LANDesk_live__?: boolean
  __TAURI__?: {
    core?: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }
    [key: string]: unknown
  }
}
