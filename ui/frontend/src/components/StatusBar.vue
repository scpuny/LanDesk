<script setup>
// 实时状态栏（FR-09.3，对齐架构 §3 ui/frontend/components/StatusBar.vue）：
// 延迟 / 编码类型 / 连接方式（内网/P2P/中继）/ 丢包率。
// 入参 `model` 为 landesk-ui `StatusBarModel`（host 经 CoreEvent 折进快照后喂前端）。
const props = defineProps({
  model: { type: Object, default: () => null },
});

function fmt(model) {
  if (!model) return null;
  return {
    state: model.state,
    connMode: model.conn_mode || "—",
    encoder: model.encoder || "—",
    latency: model.latency_ms != null ? `${model.latency_ms}ms` : "—",
    loss: model.loss_pct != null ? `${model.loss_pct}%` : "—",
    bitrate: model.bitrate_kbps != null ? `${(model.bitrate_kbps / 1000).toFixed(1)}Mbps` : "—",
    resets: model.reset_events,
  };
}
</script>

<template>
  <div class="status-bar">
    <template v-if="fmt(model)">
      <span class="state" :class="fmt(model).state">● {{ fmt(model).state }}</span>
      <span>连接 {{ fmt(model).connMode }}</span>
      <span>编码 {{ fmt(model).encoder }}</span>
      <span>延迟 {{ fmt(model).latency }}</span>
      <span>丢包 {{ fmt(model).loss }}</span>
      <span>码率 {{ fmt(model).bitrate }}</span>
      <span v-if="fmt(model).resets > 0">重置 ×{{ fmt(model).resets }}</span>
    </template>
    <template v-else>未连接</template>
  </div>
</template>

<style scoped>
.status-bar {
  display: flex; gap: 14px; align-items: center;
  padding: 6px 10px; background: rgba(0, 0, 0, 0.82); color: #eef;
  font-size: 13px; font-family: ui-monospace, monospace;
}
.state { font-weight: 700; }
.state.运行中 { color: #4caf50; }
.state.重连中 { color: #ff9800; }
.state.无可用路径, .state.鉴权失败, .state.已关闭 { color: #f44336; }
</style>
