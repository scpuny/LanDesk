// LanDesk 数据层单例：浏览器预览用 mock 数据，真实宿主走 lib/landesk.ts 桥接。
import { reactive, ref, type Ref } from 'vue'
import { landesk, detectLiveHost } from '../lib/landesk'
import type {
  DeviceRow, DeviceKind, LanDeskConfig, LogLine, LocalInfo,
} from '../types/index'

// live 判定：Tauri（withGlobalTauri 注入 __TAURI__）或显式 __LANDesk_live__===true 可同步可知；
// 浏览器无注入时异步自检 /health（detectLiveHost）定论，避免把 mock dev 服务误判为 live。
const liveNow = typeof window !== 'undefined' &&
  (!!window.__TAURI__?.core?.invoke || window.__LANDesk_live__ === true)
const isLive: Ref<boolean> = ref(liveNow)
let liveSettled: Promise<boolean> | null = null
function ensureLive(): Promise<boolean> {
  if (liveNow) return Promise.resolve(true)
  if (!liveSettled) {
    liveSettled = detectLiveHost().then((v) => {
      isLive.value = v
      return v
    })
  }
  return liveSettled
}

// ---- Mock 数据（对齐 V2 设计稿示例）----
const mockDevices: DeviceRow[] = [
  { id: '41C98B7E', name: '客厅-iMac', addr: '192.168.1.12', res: '2560×1440', enc: ['QSV'], relay: true, online: true, kind: 'monitor', lastSeen: 0 },
  { id: '9D2144AF', name: '开发板-RK3588', addr: '192.168.1.31', res: '1920×1080', enc: ['MPP'], relay: false, online: true, kind: 'chip', lastSeen: 0 },
  { id: '77BE0C13', name: '会议室大屏', addr: '192.168.1.45', res: '3840×2160', enc: ['VAAPI'], relay: true, online: true, kind: 'monitor', lastSeen: 0 },
  { id: 'DE532A90', name: '服务器-NAS', addr: '192.168.1.8', res: '1920×1080', enc: ['软编 H.264'], relay: false, online: true, kind: 'server', lastSeen: 0 },
  { id: 'B1046F8D', name: '财务工作站', addr: '192.168.1.62', res: '1920×1080', enc: [], relay: false, online: false, kind: 'laptop', lastSeen: 3 },
  { id: 'A21F09CE', name: '测试机-Ubuntu', addr: '192.168.1.77', res: '1366×768', enc: [], relay: false, online: false, kind: 'laptop', lastSeen: 12 },
]

const mockConfig: LanDeskConfig = {
  device_name: '我的工作机', password: '••••••', preset: 1, dirty_rect: true,
  mdns_enabled: true, signaling_enabled: true, signaling_url: 'ws://host:8443/ws',
  relay_enabled: true, relay_url: 'turn://relay.lan:3479',
  stun_enabled: true, stun_url: 'udp://lan:3478', log_level: 'Info',
}

const mockLogs: LogLine[] = [
  { t: '14:22:06.184', level: 'INFO', mod: 'signaling', msg: 'WebSocket 已连接', kv: 'url=ws://lan:8443/ws peer=83FA217C' },
  { t: '14:22:06.502', level: 'INFO', mod: 'ice', msg: 'NAT 类型探测完成', kv: 'type=symmetric local=192.168.1.24:51322' },
  { t: '14:22:07.418', level: 'WARN', mod: 'ice', msg: 'P2P 打洞失败，回退中继', kv: 'candidate=host~srflx timeout=300ms' },
  { t: '14:22:08.031', level: 'INFO', mod: 'session', msg: '会话建立成功', kv: 'path=relay/ice encoder=qsv 2560x1440' },
  { t: '14:22:09.240', level: 'INFO', mod: 'encoder', msg: '首帧编码完成', kv: 'keyframe=true dirty_rect=true cost=8ms' },
  { t: '14:22:12.507', level: 'DEBUG', mod: 'rtcp', msg: 'RTT 采样', kv: 'rtt=18ms jitter=1.2ms loss=0.0% bitrate=4.2Mbps' },
  { t: '14:23:41.882', level: 'WARN', mod: 'encoder', msg: '硬件编码不可用，降级 OpenH264', kv: 'reason=MFX_ERR_DEVICE_LOST reset_count=1' },
  { t: '14:26:02.114', level: 'ERROR', mod: 'auth', msg: '鉴权失败：临时密码校验不通过', kv: 'peer=41C98B7E attempts=2 nonce=ok' },
  { t: '14:31:15.706', level: 'WARN', mod: 'quick', msg: '连接空闲超时，进入重连流程', kv: 'attempt=2/5 backoff=2s' },
  { t: '14:31:18.240', level: 'INFO', mod: 'session', msg: '重连成功，强制 IDR', kv: 'path=p2p/rtp 22ms idr=true' },
]

const local = reactive<LocalInfo>({
  name: '我的工作机', deviceCode: '83FA217C', tempPassword: '629418',
  mdns: true, signaling: 'ws://lan:8443', relay: true, enc: ['QSV', 'VAAPI'], online: true,
})

const devices: Ref<DeviceRow[]> = ref<DeviceRow[]>([...mockDevices])
const deviceLoading = ref<boolean>(false)
const config = reactive<LanDeskConfig>({ ...mockConfig })
const configLoading = ref<boolean>(false)
const configError = ref<string>('')
const logs: Ref<LogLine[]> = ref<LogLine[]>([...mockLogs])

interface RawDevice {
  id: string; name: string; addr: string; res: string
  enc?: string | string[]; relay?: boolean; online?: boolean; last_seen?: number
}
function kindOf(): DeviceKind { return 'monitor' }

async function loadDevices(): Promise<void> {
  if (!(await ensureLive())) { devices.value = [...mockDevices]; return }
  deviceLoading.value = true
  try {
    const rows: RawDevice[] = await landesk.deviceList([], Math.floor(Date.now() / 1000)) || []
    devices.value = rows.map((d): DeviceRow => ({
      id: d.id, name: d.name, addr: d.addr, res: d.res,
      enc: typeof d.enc === 'string' ? d.enc.split(',').filter(Boolean) : (d.enc || []),
      relay: !!d.relay, online: d.online !== false, kind: kindOf(), lastSeen: d.last_seen || 0,
    }))
  } catch {
    devices.value = [...mockDevices]
  } finally {
    deviceLoading.value = false
  }
}

async function loadConfig(): Promise<void> {
  if (!(await ensureLive())) { Object.assign(config, mockConfig); return }
  configLoading.value = true
  configError.value = ''
  try {
    const c = await landesk.configLoadOrInit()
    if (c) Object.assign(config, c)
  } catch {
    configError.value = '配置读取失败，当前显示默认值（可能无法反映真实运行参数）'
    Object.assign(config, mockConfig)
  } finally {
    configLoading.value = false
  }
}

async function saveConfig(): Promise<void> {
  if (await ensureLive()) await landesk.configSave({ ...config })
}

function refreshTempPassword(): void {
  const n = Math.floor(100000 + Math.random() * 900000)
  local.tempPassword = String(n)
}

export interface UseLanDeskReturn {
  isLive: Ref<boolean>
  local: LocalInfo
  devices: Ref<DeviceRow[]>
  deviceLoading: Ref<boolean>
  loadDevices: () => Promise<void>
  config: LanDeskConfig
  configLoading: Ref<boolean>
  configError: Ref<string>
  loadConfig: () => Promise<void>
  saveConfig: () => Promise<void>
  logs: Ref<LogLine[]>
  refreshTempPassword: () => void
}

export function useLanDesk(): UseLanDeskReturn {
  return {
    isLive, local, devices, deviceLoading, loadDevices,
    config, configLoading, configError, loadConfig, saveConfig,
    logs, refreshTempPassword,
  }
}
