<script setup lang="ts">
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'
import type { PreviewProps, PreviewEvents } from '../types'

withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
</script>

<template>
  <div class="empty-preview">{{ PREVIEW_LABELS.empty }}</div>
</template>

<style scoped>
.empty-preview {
  margin: auto;
  color: var(--color-text-tertiary);
  font-size: 13px;
  text-align: center;
}
</style>
