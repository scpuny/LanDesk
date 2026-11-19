<script setup lang="ts">
/**
 * 通用空状态：灰蓝线性插画 + 描述文字，插画用 design token 随主题自适应。
 * 用法：<EmptyState description="未发现局域网设备" size="md" />
 */
defineProps({
  description: { type: String, default: '暂无数据' },
  size: { type: String, default: 'sm' },
})
</script>

<template>
  <div class="ll-empty" :class="`ll-empty--${size}`" role="status">
    <svg
      class="ll-empty-art"
      viewBox="0 0 132 104"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <!-- 地面阴影 -->
      <ellipse cx="66" cy="91" rx="40" ry="4.5" fill="var(--ll-color-surface-3)" />
      <!-- 从盒中飘出的文档 -->
      <g stroke="var(--ll-color-text-quaternary)" stroke-width="1.6" stroke-linejoin="round">
        <rect x="49" y="13" width="23" height="17" rx="2" fill="var(--ll-color-surface)" transform="rotate(-11 60.5 21.5)" />
        <path d="M54 20h13M54 25h9" stroke-linecap="round" />
      </g>
      <!-- 小星光 -->
      <path
        d="M96 22l1.7 4.3L102 28l-4.3 1.7L96 34l-1.7-4.3L90 28l4.3-1.7z"
        fill="var(--ll-color-surface-3)"
        stroke="var(--ll-color-text-quaternary)"
        stroke-width="1.4"
        stroke-linejoin="round"
      />
      <!-- 开口空盒：顶面开口 + 左右前壁 -->
      <g stroke="var(--ll-color-text-quaternary)" stroke-width="1.6" stroke-linejoin="round">
        <path d="M30 53L66 41l36 12-36 12z" fill="var(--ll-color-surface-3)" />
        <path d="M30 53l36 12v25L30 78z" fill="var(--ll-color-surface-2)" />
        <path d="M102 53L66 65v25l36-12z" fill="var(--ll-color-surface-2)" />
      </g>
    </svg>
    <p class="ll-empty-desc">{{ description }}</p>
    <slot name="footer" />
  </div>
</template>

<style scoped>
.ll-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--ll-space-1);
  color: var(--ll-color-text-tertiary);
  text-align: center;
}
.ll-empty--sm { padding: var(--ll-space-5) var(--ll-space-3); }
.ll-empty--md { padding: calc(var(--ll-space-6) * 2) var(--ll-space-4); }
.ll-empty-art {
  display: block;
  height: 76px;
  width: auto;
  opacity: .9;
}
.ll-empty--md .ll-empty-art { height: 108px; }
.ll-empty-desc {
  font-size: var(--ll-fs-sm);
  color: var(--ll-color-text-tertiary);
  line-height: var(--ll-lh-base);
}
.ll-empty--md .ll-empty-desc { font-size: var(--ll-fs-md); margin-top: var(--ll-space-1); }
</style>
