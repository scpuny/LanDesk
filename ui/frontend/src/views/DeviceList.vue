<script setup>
// 设备列表视图（FR-01.2 局域网一键发现 + FR-01.4 离线移除，对齐架构 §3 ui/frontend）。
// 数据经 landesk.deviceList() 取（后端 mdns 监听 + TTL 剪枝 + 排序，出参 DeviceRow[]）。
import { ref, onMounted } from "vue";
import { landesk } from "../lib/landesk.js";

const devices = ref([]);
const nowSecs = ref(0);
const selected = ref(null);

// mDNS 监听在 Tauri 宿主侧（Rust `Listener`）跑；前端定时刷新列表。
async function refresh() {
  // 后端应暴露 `list_devices`（或经 mdns Listener 推送）。此处经现有 device_list 命令
  // 演示：宿主侧持在线设备 Vec<OnlineDevice>，前端传 now 秒。联调时由宿主喂入。
  const online = await window.__LANDesk_online__?.() ?? [];
  const rows = await landesk.deviceList(online, nowSecs.value);
  devices.value = rows;
}

function pick(d) {
  selected.value = d;
  // 一键连接（FR-01.3 内网）→ 走 host 侧 Controller.connect + choose_path 决策。
  window.__LANDesk_connect__?.(d);
}

onMounted(async () => {
  nowSecs.value = Math.floor(Date.now() / 1000);
  try {
    await refresh();
  } catch (e) {
    devices.value = []; // 非 Tauri / 无监听时降级空表
  }
});

setInterval(refresh, 5000); // TTL 45s 内每 5s 刷新，过期设备自动消失（FR-01.4）
</script>

<template>
  <div class="device-list">
    <h2>局域网在线设备（FR-01.2）</h2>
    <p v-if="devices.length === 0" class="empty">未发现设备（需 Tauri 宿主 + mDNS 监听）</p>
    <ul>
      <li v-for="d in devices" :key="d.id" :class="{ active: selected?.id === d.id }" @click="pick(d)">
        <strong>{{ d.name }}</strong>
        <span class="addr">{{ d.addr }}</span>
        <span class="res">{{ d.res }}</span>
        <span v-if="d.relay" class="badge">中继</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.device-list { padding: 12px; }
.empty { color: #888; }
ul { list-style: none; padding: 0; }
li { padding: 8px; border-radius: 6px; cursor: pointer; display: flex; gap: 10px; align-items: center; }
li:hover, li.active { background: rgba(0, 0, 0, 0.06); }
.addr { color: #555; font-family: monospace; }
.res { color: #888; font-size: 12px; }
.badge { background: #2f6fed; color: #fff; border-radius: 4px; padding: 0 6px; font-size: 11px; }
</style>
