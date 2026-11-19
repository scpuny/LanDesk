<script setup>
// 受控端配置面板（FR-08，对齐架构 §3 ui/frontend/views/Settings.vue）：
// 访问密码、设备名、mDNS/信令/中继开关、STUN/信令地址、编码预设、日志级别。
// 读/写经 landesk.configLoadOrInit / configSave（landesk-core Config 的 JSON 往返）。
import { ref, onMounted } from "vue";
import { landesk } from "../lib/landesk.js";

const cfg = ref(null);
const saving = ref(false);
const preset = ref("Balanced");

async function load() {
  try {
    cfg.value = await landesk.configLoadOrInit();
  } catch (e) {
    cfg.value = null; // 非 Tauri 环境降级
  }
}

async function save() {
  if (!cfg.value) return;
  cfg.value.preset = preset.value;
  saving.value = true;
  try {
    await landesk.configSave(cfg.value);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="settings" v-if="cfg">
    <h2>配置（FR-08）</h2>

    <label>设备名 <input v-model="cfg.device_name" maxlength="32" /></label>

    <label>访问密码 <input type="password" v-model="cfg.password" placeholder="留空 = 不设置" /></label>

    <fieldset>
      <legend>连接方式（开关）</legend>
      <label><input type="checkbox" v-model="cfg.mdns_enabled" /> mDNS 局域网广播（FR-01）</label>
      <label><input type="checkbox" v-model="cfg.signaling_enabled" /> 外网信令注册（FR-10，默认关 NFR-03）</label>
      <label><input type="checkbox" v-model="cfg.relay_enabled" /> 中继兜底（FR-11.2，可关 → 打不通即拒绝）</label>
    </fieldset>

    <label>信令地址 <input v-model="cfg.signaling_url" placeholder="ws://host:8443/ws" /></label>
    <label>STUN 地址 <input v-model="cfg.stun_url" placeholder="udp://host:3478" /></label>

    <label>编码预设
      <select v-model="preset">
        <option value="Smooth">流畅（1.5M）</option>
        <option value="Balanced">平衡（4M）</option>
        <option value="Hd">高清（8M）</option>
      </select>
    </label>

    <label>脏矩形 <input type="checkbox" v-model="cfg.dirty_rect" />（关则全帧，风险表#3）</label>

    <label>日志级别
      <select v-model="cfg.log_level">
        <option>Info</option>
        <option>Debug</option>
        <option>Warn</option>
        <option>Error</option>
      </select>
    </label>

    <button :disabled="saving" @click="save">{{ saving ? "保存中…" : "保存配置" }}</button>
  </div>
  <p v-else class="empty">配置不可用（需 Tauri 宿主）</p>
</template>

<style scoped>
.settings { padding: 12px; display: flex; flex-direction: column; gap: 10px; max-width: 480px; }
label { display: flex; align-items: center; gap: 8px; }
input, select { padding: 4px 6px; }
fieldset { display: flex; flex-direction: column; gap: 4px; padding: 8px; }
.empty { color: #888; }
button { margin-top: 8px; padding: 6px 14px; }
</style>
