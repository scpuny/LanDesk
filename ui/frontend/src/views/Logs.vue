<script setup lang="ts">
// 日志页：深色终端风格 + 级别筛选 + 关键字过滤，对齐 V2 设计稿。
// P2.8：live 宿主走后端 logQuery/logClear（环形缓冲），非 live 保留 mock 预览 + 客户端过滤。
import { ref, computed, onMounted, watch } from 'vue'
import { toast } from '../composables/useUi'
import { useLanDesk } from '../composables/useLanDesk'
import type { LogFilter } from '../types/index'
import Icon from '../components/common/Icon.vue'

const { isLive, logs, loadLogs, clearLogs } = useLanDesk()
const level = ref<LogFilter>('ALL')
const keyword = ref('')
const autoScroll = ref(true)
const levelOptions: LogFilter[] = ['ALL', 'INFO', 'WARN', 'ERROR', 'DEBUG']

const counts = computed((): Record<LogFilter, number> => ({
  ALL: logs.value.length,
  INFO: logs.value.filter((l) => l.level === 'INFO').length,
  WARN: logs.value.filter((l) => l.level === 'WARN').length,
  ERROR: logs.value.filter((l) => l.level === 'ERROR').length,
  DEBUG: logs.value.filter((l) => l.level === 'DEBUG').length,
}))
const filtered = computed(() => {
  const q = keyword.value.trim().toLowerCase()
  return logs.value.filter((l) =>
    (level.value === 'ALL' || l.level === level.value) &&
    (!q || `${l.mod} ${l.msg} ${l.kv}`.toLowerCase().includes(q)))
})
// 清空：live 调后端 logClear（环形缓冲归零）并同步本地；非 live 仅清本地。
async function clear() { await clearLogs(); toast('日志已清空', 'info') }
function exportLog() { toast('已导出 landsdk.log', 'success') }

// 拉取日志：live 走后端环形缓冲，非 live 保留 mock。
onMounted(() => { loadLogs(level.value, keyword.value || undefined) })
// live 下筛选变化 → 重新查询后端（客户端 filtered 仍即时作用于已拉快照，双重一致）。
watch([level, keyword], () => { if (isLive.value) loadLogs(level.value, keyword.value || undefined) })
</script>

<template>
  <div class="ld-content">
    <!-- 工具栏 -->
    <div class="ld-card" style="padding:16px">
      <div class="ld-log-tools">
        <div class="ll-input-wrap" style="width:140px">
          <Icon name="filter" :size="14" style="position:absolute;left:10px;color:var(--ll-color-text-tertiary)" />
          <select v-model="level" class="ll-input" style="padding-left:32px">
            <option value="ALL">全部级别</option>
            <option value="INFO">INFO</option>
            <option value="WARN">WARN</option>
            <option value="ERROR">ERROR</option>
            <option value="DEBUG">DEBUG</option>
          </select>
        </div>
        <div class="ll-input-wrap ld-log-tools__search">
          <Icon name="search" :size="14" style="position:absolute;left:10px;color:var(--ll-color-text-tertiary)" />
          <input v-model="keyword" class="ll-input" style="padding-left:32px" placeholder="过滤关键字" />
        </div>
        <span class="ld-switch-label"><button class="ll-switch" :data-on="autoScroll" @click="autoScroll = !autoScroll" /> 自动滚动</span>
        <div style="flex:1" />
        <button class="ll-btn ll-btn--default" @click="exportLog"><Icon name="download" :size="14" /> 导出 log</button>
        <button class="ll-btn ll-btn--danger-outline" @click="clear"><Icon name="trash" :size="14" /> 清空</button>
      </div>
      <div class="ld-log-filters" style="margin-top:12px">
        <button v-for="lv in levelOptions" :key="lv" class="ld-log-filter" :class="{ 'is-active': level === lv }" @click="level = lv">
          <span v-if="lv !== 'ALL'" class="ll-dot" :class="{ 'll-dot--brand': lv==='INFO', 'll-dot--warning': lv==='WARN', 'll-dot--danger': lv==='ERROR' }" />
          {{ lv === 'ALL' ? '全部' : lv }} {{ counts[lv] }}
        </button>
        <span class="ld-form-help" style="margin-left:auto">已保留最近 2000 条日志（环形缓冲）</span>
      </div>
    </div>

    <!-- 终端 -->
    <div class="ld-terminal">
      <div class="ld-terminal__bar">
        <span class="ld-dots"><i style="background:#ff5f57"/><i style="background:#febc2e"/><i style="background:#28c840"/></span>
        <span class="ld-mono">landsdk.log · 实时输出</span>
        <span class="live"><span class="ll-dot ll-dot--success" /> LIVE</span>
        <span>UTF-8</span><span>CRLF</span>
      </div>
      <div class="ld-log-list">
        <div v-for="(l, i) in filtered" :key="i" class="ld-log-line" :class="`is-${l.level}`">
          <span class="ld-log-time">{{ l.t }}</span>
          <span class="ld-log-level" :class="`is-${l.level}`">{{ l.level }}</span>
          <span class="ld-log-mod">{{ l.mod }}</span>
          <span class="ld-log-msg">{{ l.msg }} <span class="ld-log-kv">{{ l.kv }}</span></span>
        </div>
        <div v-if="filtered.length === 0" style="padding:48px;text-align:center;color:var(--ll-color-text-tertiary)">
          <Icon name="search" :size="26" />
          <p style="margin-top:12px">没有匹配「{{ keyword }}」的日志</p>
        </div>
      </div>
      <div class="ld-terminal__foot">
        <span>F1 : {{ filtered.length }} / 2000</span>
        <span style="margin-left:16px">环形缓冲已保留最近 2000 条</span>
        <span class="r">↓ 已暂停新日志</span>
      </div>
    </div>
  </div>
</template>
