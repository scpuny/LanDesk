<script setup lang="ts">
// 设置页：横向表单（label 左对齐固定宽 140px + 两列网格 + 长文本跨双列），对齐 V2 设计稿。
import { ref, computed, onMounted } from 'vue'
import { toast, confirm } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import type { Preset, LanDeskConfig } from '../types/index'
import Icon from '../components/common/Icon.vue'

const { config, configLoading, configError, loadConfig, saveConfig } = useLanDesk()
const dirty = ref(false)
const saving = ref(false)
const showPassword = ref(false)

const presets: { v: Preset; label: string; sub: string }[] = [
  { v: 0, label: '流畅 1.5M', sub: '低带宽 / 优先流畅' },
  { v: 1, label: '平衡 4M', sub: '画质与延迟平衡' },
  { v: 2, label: '高清 8M', sub: '高画质 / 局域网推荐' },
]
const logLevels: LanDeskConfig['log_level'][] = ['Info', 'Debug', 'Warn', 'Error']

/** 临时密码通道开关：`undefined` 视作默认开（与后端 `serde(default=true)` 一致）。 */
const tempEnabled = computed({
  get: () => config.temp_password_enabled ?? true,
  set: (v: boolean) => {
    config.temp_password_enabled = v
  },
})
const tempTtlOptions = [
  { v: 300, label: '5 分钟' },
  { v: 900, label: '15 分钟' },
  { v: 3600, label: '1 小时' },
]

function markDirty() { dirty.value = true }
function pickPreset(v: Preset) { config.preset = v; markDirty() }
function pickLog(l: LanDeskConfig['log_level']) { config.log_level = l; markDirty() }
async function save() {
  saving.value = true
  try { await saveConfig(); dirty.value = false; toast('配置已保存') }
  catch { toast('保存失败', 'error') }
  finally { saving.value = false }
}
async function reset() {
  const ok = await confirm({ title: '重置配置', message: '确定放弃未保存的更改，恢复上次保存的配置吗？', confirmText: '重置', tone: 'warning' })
  if (ok) { await loadConfig(); dirty.value = false; toast('已恢复上次保存的配置', 'info') }
}

onMounted(loadConfig)
</script>

<template>
  <div class="ld-content">
    <!-- 未保存提示在顶栏由 App 读取；此处错误横幅 -->
    <div v-if="configError" class="ld-banner-error">
      <Icon name="alertCircle" :size="16" /> {{ configError }}
      <button class="ll-btn ll-btn--text ll-btn--sm" style="margin-left:auto" @click="loadConfig()"><Icon name="refresh" :size="13" /> 重试</button>
    </div>

    <div v-if="configLoading" class="ld-card ld-card__pad">
      <div class="ll-skeleton" style="height:24px;width:30%;margin-bottom:24px" />
      <div v-for="i in 5" :key="i" class="ll-skeleton" style="height:36px;margin-bottom:12px" />
    </div>

    <template v-else>
      <!-- 基础信息 -->
      <section class="ld-card ld-card__pad">
        <div class="ld-card__head">
          <Icon name="info" :size="18" style="color:var(--ll-color-brand)" />
          <div><div class="ld-card__title">基础信息</div><div class="ld-card__sub">设备标识 · 访问密码与画质预设</div></div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">设备名称<small>最多 32 字符 · 局域网内展示</small></div>
          <div class="ld-form-row__control">
            <input v-model="config.device_name" class="ll-input" maxlength="32" @input="markDirty" />
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">访问密码<small>仅在本机生成，不保存明文</small></div>
          <div class="ld-form-row__control">
            <div class="ll-input-wrap" style="width:260px">
              <input v-model="config.password" :type="showPassword ? 'text' : 'password'" class="ll-input" @input="markDirty" />
              <span class="ll-input-affix ll-click" @click="showPassword = !showPassword"><Icon :name="showPassword ? 'eyeOff' : 'eye'" :size="15" /></span>
            </div>
            <span class="ld-form-help">设置密码后，本机可被局域网内其他设备用「访问密码」连接（长期身份凭证）。留空也可被连——见下方「临时密码通道」。</span>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">临时密码通道<small>一次性配对码</small></div>
          <div class="ld-form-row__control">
            <button class="ll-switch" :data-on="tempEnabled" @click="tempEnabled = !tempEnabled; markDirty()" />
            <span class="ld-form-help">开启后本机可随时生成 6 位临时码，给「临时放别人进来」用；与访问密码是<strong>两道独立门</strong>，任一通过即可连接。默认开启。</span>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">临时码有效期<small>到期自动换新</small></div>
          <div class="ld-form-row__control">
            <button v-for="o in tempTtlOptions" :key="o.v" class="ld-chip-opt" :class="{ 'is-active': config.temp_password_ttl_secs === o.v }" :disabled="!tempEnabled" @click="config.temp_password_ttl_secs = o.v; markDirty()">
              {{ o.label }}
            </button>
            <span class="ld-form-help">到达有效期后受控端自动换新码，旧码作废——临时授权不会长期留口。</span>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">画质预设</div>
          <div class="ld-form-row__control">
            <button v-for="p in presets" :key="p.v" class="ld-chip-opt" :class="{ 'is-active': config.preset === p.v }" @click="pickPreset(p.v)">
              {{ p.label }}
            </button>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">脏矩形编码</div>
          <div class="ld-form-row__control">
            <button class="ll-switch" :data-on="config.dirty_rect" @click="config.dirty_rect = !config.dirty_rect; markDirty()" />
            <span class="ld-switch-label">{{ config.dirty_rect ? '已开启' : '已关闭' }}</span>
            <span class="ld-form-help">仅传输变化区域，静止画面几乎不产生数据流量</span>
          </div>
        </div>
      </section>

      <!-- 连接方式 -->
      <section class="ld-card ld-card__pad">
        <div class="ld-card__head">
          <Icon name="network" :size="18" style="color:var(--ll-color-brand)" />
          <div><div class="ld-card__title">连接方式</div><div class="ld-card__sub">发现、信令、中继与 STUN 参数</div></div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">mDNS 发现<small>局域网自动发现</small></div>
          <div class="ld-form-row__control">
            <button class="ll-switch" :data-on="config.mdns_enabled" @click="config.mdns_enabled = !config.mdns_enabled; markDirty()" />
            <span class="ld-form-help">开启后可在局域网自动发现同网段设备</span>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">信令服务<small>外网设备码连接</small></div>
          <div class="ld-form-row__control">
            <button class="ll-switch" :data-on="config.signaling_enabled" @click="config.signaling_enabled = !config.signaling_enabled; markDirty()" />
            <span class="ld-form-help">开启后需填写信令地址</span>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">信令地址</div>
          <div class="ld-form-row__control">
            <input v-model="config.signaling_url" class="ll-input" :class="{ 'll-input--error': config.signaling_enabled && !config.signaling_url }" placeholder="ws://host:8443/ws" @input="markDirty" />
            <span v-if="config.signaling_enabled && !config.signaling_url" class="ld-form-error">已开启信令服务，地址不可为空</span>
          </div>
        </div>
        <div class="ld-form-row ld-form-row--two">
          <div class="ld-form-row__label">中继 / STUN</div>
          <div class="ld-form-row__control" style="flex-direction:column;align-items:stretch">
            <div style="display:flex;align-items:center;gap:8px">
              <button class="ll-switch" :data-on="config.relay_enabled" @click="config.relay_enabled = !config.relay_enabled; markDirty()" />
              <span class="ld-switch-label">中继服务</span>
            </div>
            <input v-model="config.relay_url" class="ll-input" placeholder="turn://relay.lan:3479" @input="markDirty" />
            <span class="ld-form-help">P2P 打洞失败时自动回落中继</span>
          </div>
          <div class="ld-form-row__control" style="flex-direction:column;align-items:stretch">
            <div style="display:flex;align-items:center;gap:8px">
              <button class="ll-switch" :data-on="config.stun_enabled" @click="config.stun_enabled = !config.stun_enabled; markDirty()" />
              <span class="ld-switch-label">STUN 地址</span>
            </div>
            <input v-model="config.stun_url" class="ll-input" placeholder="udp://host:3478" @input="markDirty" />
            <span class="ld-form-help">用于探测 NAT 类型与公网映射</span>
          </div>
        </div>
      </section>

      <!-- 日志诊断 -->
      <section class="ld-card ld-card__pad">
        <div class="ld-card__head">
          <Icon name="file" :size="18" style="color:var(--ll-color-brand)" />
          <div><div class="ld-card__title">日志诊断</div><div class="ld-card__sub">日志级别与编码器参数说明</div></div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">日志级别</div>
          <div class="ld-form-row__control">
            <button v-for="l in logLevels" :key="l" class="ld-chip-opt" :class="{ 'is-active': config.log_level === l }" @click="pickLog(l)">{{ l }}</button>
          </div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">编码器日志</div>
          <div class="ld-form-row__control"><span class="ll-link" style="font-size:13px">跳转到日志中心并带 encoder 关键字筛选 ›</span></div>
        </div>
        <div class="ld-form-row">
          <div class="ld-form-row__label">编码参数说明</div>
          <div class="ld-form-row__control" style="grid-column:2/-1">
            <div class="ld-note-box">当前编码优先级：QSV → VAAPI → OpenH264 软编。硬件编码不可用时自动降级，会话不中断，重置次数 +1。脏矩形编码仅发送变化区域；关键帧间隔 2 秒；码率上限由画质预设决定（流畅 1.5M / 平衡 4M / 高清 8M）。</div>
          </div>
        </div>
      </section>

      <!-- 操作条 -->
      <div class="ld-form-actions">
        <span class="ld-form-actions__hint"><Icon name="info" :size="14" /> 保存后立即生效，已进行的会话不受影响</span>
        <div class="ld-form-actions__btns">
          <button class="ll-btn ll-btn--default" @click="reset">重置</button>
          <button class="ll-btn ll-btn--primary" :disabled="saving" @click="save">
            <span v-if="saving" class="ll-spinner" /> {{ saving ? '保存中' : '保存配置' }}
          </button>
        </div>
      </div>
    </template>
  </div>
</template>
