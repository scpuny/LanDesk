<script setup>
// 日志 / 编码器日志查看（FR-08.4，对齐架构 §3 ui/frontend/views/Logs.vue + NFR-06）。
// 编码器日志单独落 `landesk-encoder.log`（NFR-06）；UI 读文件流实时刷新。
// 宿主侧经 Tauri event 推日志行（tracing 落盘 → 读文件增量）；此处订阅 `log-line`。
import { ref, onMounted, onBeforeUnmount } from "vue";

const lines = ref([]);
const filter = ref("");
let unlisten = null;

const shown = () =>
  lines.value.filter((l) =>
    !filter.value ? true : l.toLowerCase().includes(filter.value.toLowerCase()),
  );

onMounted(async () => {
  const tauri = window.__TAURI__;
  if (tauri?.event?.listen) {
    unlisten = await tauri.event.listen("log-line", (e) => {
      lines.value.push(e.payload);
      if (lines.value.length > 2000) lines.value.shift(); // 环形缓冲，防内存涨（NFR-01）
    });
  }
});

onBeforeUnmount(() => unlisten?.());
</script>

<template>
  <div class="logs">
    <h2>日志 / 编码器日志（FR-08.4 / NFR-06）</h2>
    <input v-model="filter" placeholder="过滤（如 warn / qsv / 打洞）" />
    <pre class="pane">{{ shown().join("\n") }}</pre>
    <p v-if="lines.length === 0" class="empty">暂无日志（需 Tauri 宿主订阅事件流）</p>
  </div>
</template>

<style scoped>
.logs { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
.pane { background: #111; color: #d8e3ff; padding: 8px; border-radius: 6px; height: 60vh; overflow: auto; font-size: 12px; }
input { padding: 4px 8px; }
.empty { color: #888; }
</style>
