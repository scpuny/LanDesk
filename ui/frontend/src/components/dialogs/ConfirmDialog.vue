<script setup lang="ts">
// 危险操作二次确认弹窗：由 useUi().confirm() 的 Promise 驱动。
import { useUi } from '../../composables/useUi'
import Modal from '../common/Modal.vue'
import Icon from '../common/Icon.vue'

const { confirmRequest, resolveConfirm } = useUi()
</script>

<template>
  <Modal
    v-if="confirmRequest"
    :title="confirmRequest.title"
    icon="alertTriangle"
    :badge="confirmRequest.tone === 'danger' ? '不可撤销' : ''"
    badge-variant="danger"
    :width="440"
    @close="resolveConfirm(false)"
  >
    <p style="font-size:var(--ll-fs-sm);line-height:1.7;color:var(--ll-color-text-secondary);white-space:pre-line">{{ confirmRequest.message }}</p>
    <template #footer>
      <div style="display:flex;justify-content:flex-end;gap:var(--ll-space-2)">
        <button class="ll-btn ll-btn--default" @click="resolveConfirm(false)">{{ confirmRequest.cancelText }}</button>
        <button class="ll-btn" :class="confirmRequest.tone === 'danger' ? 'll-btn--danger' : 'll-btn--primary'" @click="resolveConfirm(true)">
          <Icon name="check" :size="14" /> {{ confirmRequest.confirmText }}
        </button>
      </div>
    </template>
  </Modal>
</template>
