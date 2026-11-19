<script setup>
// 会话预览窗口（FR-09，对齐架构 §3 ui/frontend/views/Session.vue）：
// 自适应/原始尺寸 + 全屏 + 状态栏（StatusBar）。画面本身由 host 侧采集→编码→
// 解码渲染（M1/M2/M3）；本视图只管「缩放/全屏/状态栏」UI 层，画布交给宿主 canvas。
import { ref } from "vue";
import StatusBar from "../components/StatusBar.vue";

const status = ref(null); // landesk-ui StatusBarModel（host 经事件通道喂）
const scaleMode = ref("fit"); // "fit" | "original"
const fullscreen = ref(false);

function toggleFullscreen() {
  fullscreen.value = !fullscreen.value;
  // 宿主侧按 fullscreen 切换窗口/浏览器全屏（联调）。
  window.__LANDesk_setFullscreen__?.(fullscreen.value);
}
</script>

<template>
  <div class="session" :class="{ fullscreen }">
    <!-- 画面：宿主渲染 H.264 解码帧到此 canvas（M3 渲染层） -->
    <canvas
      class="video"
      :style="{ transform: scaleMode === 'fit' ? 'scale(1)' : 'none' }"
      @click="toggleFullscreen"
    ></canvas>

    <StatusBar :model="status" />

    <div class="toolbar">
      <button @click="scaleMode = 'fit'">自适应</button>
      <button @click="scaleMode = 'original'">原始尺寸</button>
      <button @click="toggleFullscreen">{{ fullscreen ? "退出全屏" : "全屏" }}</button>
    </div>
  </div>
</template>

<style scoped>
.session { position: relative; display: flex; flex-direction: column; height: 100%; }
.video { flex: 1; background: #000; width: 100%; }
.toolbar { display: flex; gap: 8px; padding: 8px; }
.toolbar button { padding: 4px 10px; }
.fullscreen { position: fixed; inset: 0; z-index: 100; }
</style>
