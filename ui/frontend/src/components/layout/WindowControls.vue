<script setup lang="ts">
// 自绘主窗体窗口控制三键（最小化 / 最大化·还原 / 关闭），配合去系统标题栏。
// 平台感知：macOS 保留系统交通灯（`titleBarStyle:overlay`）→ 本组件不渲染（父顶栏左侧留白）；
// Windows / Linux 全自绘（`decorations:false`）→ 渲染三键。浏览器预览无窗口可控 → 隐藏。
// 按钮**不**标 `data-tauri-drag-region`（可点击元素须排除拖拽，Tauri drag.js 约定）。
import { useWindowControls } from '../../composables/useWindowControls'
import Icon from '../common/Icon.vue'

const { showControls, onMinimize, onMaximize, onClose } = useWindowControls()
</script>

<template>
  <div v-if="showControls" class="ld-win-ctrl" role="group" aria-label="窗口控制">
    <button class="ld-win-ctrl__btn" title="最小化" aria-label="最小化" @click="onMinimize">
      <Icon name="minus" :size="14" />
    </button>
    <button class="ld-win-ctrl__btn" title="最大化" aria-label="最大化 / 还原" @click="onMaximize">
      <Icon name="winMax" :size="13" />
    </button>
    <button class="ld-win-ctrl__btn ld-win-ctrl__btn--close" title="关闭" aria-label="关闭" @click="onClose">
      <Icon name="close" :size="14" />
    </button>
  </div>
</template>

<style scoped>
/* 自绘窗口三键：Win/Linux 习惯（右侧 min/max/close，46×32 点击热区）。macOS 不渲染此组件。
   色值消费 tokens；关闭键 hover 走危险色（对齐 Windows）。 */
.ld-win-ctrl { display: inline-flex; align-items: center; height: 100%; }
.ld-win-ctrl__btn {
  width: 46px; height: 100%; display: inline-flex; align-items: center; justify-content: center;
  border: none; background: transparent; color: var(--ll-color-text-secondary); cursor: pointer;
  transition: background var(--ll-duration-fast), color var(--ll-duration-fast);
}
.ld-win-ctrl__btn:hover { background: var(--ll-color-surface-hover); color: var(--ll-color-text); }
.ld-win-ctrl__btn--close:hover {
  background: var(--ll-color-danger);
  color: var(--ll-color-text-inverse, #fff);
}
</style>
