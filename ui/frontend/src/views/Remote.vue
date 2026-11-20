<script setup lang="ts">
// 远程控制页（菜单置顶）：快速连接 + 本机设备码/临时密码 + 本机运行信息。
// 从原 DeviceList 拆出「发起侧 + 本机信息」职责；「我的设备」页保留局域网设备列表（选择侧）。
// 发起远程控制/一次性密码：live(Tauri) 打开独立会话 OS 窗体并带目标；非 live 浏览器预览走单页 session。
import { ref, onMounted } from 'vue'
import { useUi, toast } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import { launchSession } from '../composables/useSessionWindow'
import Icon from '../components/common/Icon.vue'

const { navigate } = useUi()
const { local, refreshTempPassword, loadLocal } = useLanDesk()
const quickCode = ref('41C98B7E')
const history = ['9D21 44AF', '77BE 0C13', 'DE53 2A90', 'B104 6F8D']

/** 发起远程控制：把设备码作为目标交给会话窗体（Tauri 独立 OS 窗 / 浏览器单页）。 */
function remoteControl() {
  const code = quickCode.value.replace(/\s/g, '')
  if (!code) { toast('请输入对方设备码', 'warning'); return }
  launchSession({ name: code, id: code, res: '1920×1080' }, 'p2p')
}
function oneTimePassword() {
  const code = quickCode.value.replace(/\s/g, '')
  if (!code) { toast('请输入对方设备码', 'warning'); return }
  toast('本次会话使用一次性临时密码', 'info')
  launchSession({ name: code, id: code, res: '1920×1080' }, 'p2p')
}
function copy(text: string, label = '已复制') {
  navigator.clipboard?.writeText(text).then(() => toast(label)).catch(() => toast(label))
}

onMounted(() => { loadLocal() })
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
          <input v-model="quickCode" class="ll-input ll-input--mono ld-quick__code" placeholder="请输入对方 8 位设备码" maxlength="12" @keyup.enter="remoteControl" />
          <span class="ld-quick__hint">8 位设备码</span>
        </div>
        <button class="ll-btn ll-btn--primary ld-quick__btn" @click="remoteControl"><Icon name="link" :size="15" /> 远程控制</button>
        <button class="ll-btn ll-btn--default ld-quick__btn" @click="oneTimePassword"><Icon name="key" :size="15" /> 一次性密码</button>
      </div>
      <div class="ld-recent">
        <span class="ld-recent__label">最近连接</span>
        <button v-for="h in history" :key="h" class="ld-chip" @click="quickCode = h.replace(/\s/g,'')">
          <Icon name="history" :size="12" /> {{ h }}
        </button>
      </div>
    </section>

    <!-- 下方两栏：左（本机设备码 + 运行信息）｜右（本机运行信息） -->
    <div class="ld-device-columns">
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
      </aside>

      <!-- 右栏：本机运行信息（白底，独立） -->
      <section class="ld-dev-section">
        <section class="ld-card ld-card__pad ld-info-card">
          <div class="ld-info-card__title">本机运行信息</div>
          <div class="ld-kvline"><span class="ld-kvline__k">mDNS 发现</span><span class="ld-kvline__v"><span class="ll-dot" :class="local.mdns ? 'll-dot--success' : 'll-dot--idle'" /> {{ local.mdns ? '已启用' : '未启用' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">信令服务</span><span class="ld-kvline__v ld-mono">{{ local.signaling || '未配置' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">中继服务</span><span class="ld-kvline__v"><span class="ll-dot" :class="local.relay ? 'll-dot--success' : 'll-dot--idle'" /> {{ local.relay ? '可用' : '不可用' }}</span></div>
          <div class="ld-kvline"><span class="ld-kvline__k">编码能力</span><span class="ld-kvline__v"><span v-if="local.enc.length" class="ll-tag">{{ local.enc.join(' · ') }}</span><span v-else class="ld-form-help">未探测</span></span></div>
          <div class="ll-divider" />
          <div class="ld-sess-actions">
            <button class="ll-btn ll-btn--default" @click="navigate('settings')"><Icon name="settings" :size="14" /> 受控设置</button>
            <button class="ll-btn ll-btn--default" @click="navigate('logs')"><Icon name="file" :size="14" /> 运行日志</button>
          </div>
        </section>
      </section>
    </div>
  </div>
</template>
