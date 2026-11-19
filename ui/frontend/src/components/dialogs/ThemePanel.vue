<script setup lang="ts">
// 主题切换面板：明暗（light/dark/system）+ 视觉风格（6 套 lanLink 风格）
// 6 套风格卡片 + 标题右侧「原型基准」切换按钮；选中即切换，无确认按钮。
import { computed } from 'vue'
import { useTheme } from '../../composables/useTheme'
import { VISUAL_STYLES, DEFAULT_PALETTE } from '../../data/themes'
import type { ThemePref } from '../../types/index'
import Icon from '../common/Icon.vue'

defineProps({ open: { type: Boolean, default: false } })

const { pref, setTheme, palette, setPalette } = useTheme()
const modes: { id: ThemePref; label: string; icon: string }[] = [
  { id: 'light', label: '浅色', icon: 'sun' },
  { id: 'dark', label: '深色', icon: 'moon' },
  { id: 'system', label: '跟随系统', icon: 'settings' },
]
const activePalette = computed(() => palette.value || DEFAULT_PALETTE)
const isBaseline = computed(() => activePalette.value === DEFAULT_PALETTE)
</script>

<template>
  <div v-if="open" class="ld-theme-pop">
    <div class="ld-theme-pop__title">外观 · 明暗模式</div>
    <div class="ll-segmented ld-theme-modes">
      <button v-for="m in modes" :key="m.id" class="ld-theme-mode"
        :data-active="pref === m.id" @click="setTheme(m.id)">
        <Icon :name="m.icon" :size="13" /><span>{{ m.label }}</span>
      </button>
    </div>

    <div class="ld-theme-head">
      <span class="ld-theme-pop__title ld-theme-head__title">视觉风格</span>
      <button class="ld-baseline-btn" :class="{ 'is-active': isBaseline }"
        :title="'切换到原型基准（' + '#2F6FED' + ' 蓝色系）'" @click="setPalette(DEFAULT_PALETTE)">
        <Icon name="palette" :size="13" /><span>原型基准</span>
      </button>
    </div>
    <div class="ld-theme-grid">
      <button v-for="p in VISUAL_STYLES" :key="p.id" class="ld-theme-card" :class="{ 'is-active': activePalette === p.id }"
        :title="p.desc" @click="setPalette(p.id)">
        <span class="ld-swatch"><i :style="{ background: p.swatch[0] }"/><i :style="{ background: p.swatch[1] }"/><i :style="{ background: p.swatch[2] }"/></span>
        <span>{{ p.short }}</span>
      </button>
    </div>
  </div>
</template>
