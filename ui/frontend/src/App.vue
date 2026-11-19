<script setup>
// LanDesk 主壳（FR-09 预览 + 设备列表 + 设置 + 日志，对齐架构 §3 ui/frontend）。
// 4 个视图 Tab 切换；StatusBar 由 host 经事件喂 StatusBarModel。
import { ref } from "vue";
import DeviceList from "./views/DeviceList.vue";
import Session from "./views/Session.vue";
import Settings from "./views/Settings.vue";
import Logs from "./views/Logs.vue";

const tab = ref("devices"); // devices | session | settings | logs
const status = ref(null); // landesk-ui StatusBarModel（host 事件喂）

const tabs = [
  ["devices", "设备"],
  ["session", "会话"],
  ["settings", "设置"],
  ["logs", "日志"],
];
</script>

<template>
  <div class="shell">
    <nav>
      <button v-for="[k, label] in tabs" :key="k" :class="{ on: tab === k }" @click="tab = k">
        {{ label }}
      </button>
    </nav>
    <main>
      <DeviceList v-if="tab === 'devices'" />
      <Session v-if="tab === 'session'" :status="status" />
      <Settings v-if="tab === 'settings'" />
      <Logs v-if="tab === 'logs'" />
    </main>
  </div>
</template>

<style>
html, body { margin: 0; height: 100%; font-family: system-ui, sans-serif; }
.shell { display: flex; height: 100%; }
nav { width: 120px; display: flex; flex-direction: column; gap: 4px; padding: 8px; }
nav button { padding: 8px; text-align: left; border: 0; background: none; cursor: pointer; }
nav button.on { background: #2f6fed; color: #fff; border-radius: 6px; }
main { flex: 1; overflow: auto; }
</style>
