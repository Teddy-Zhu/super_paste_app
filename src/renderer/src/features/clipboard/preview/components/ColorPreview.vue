<script setup lang="ts">
import { ref, watch } from 'vue'
import { clipboard } from '@renderer/api/clipboard'
import { showMessage } from '@renderer/api/dialog'
import type { PreviewProps, PreviewEvents } from '../types'

const props = withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
const copiedFormat = ref('')
const copying = ref(false)
watch(
  () => props.preview.color,
  () => (copiedFormat.value = '')
)
const copyColor = async (format: 'hex' | 'rgb'): Promise<void> => {
  const color = props.preview.colorFormats
  if (!color || copying.value) return
  copying.value = true
  copiedFormat.value = ''
  try {
    await clipboard.copyColor(color[format])
    if (props.preview.colorFormats === color) copiedFormat.value = format.toUpperCase()
  } catch (error) {
    await showMessage('复制颜色失败: ' + (error as Error).message)
  } finally {
    copying.value = false
  }
}
</script>

<template>
  <template v-if="preview.color">
    <div class="color-swatch checkerboard">
      <div :style="{ backgroundColor: preview.color }"></div>
    </div>
    <code class="color-value">{{ preview.color }}</code>
    <code v-if="preview.colorFormats" class="color-rgb">{{ preview.colorFormats.rgb }}</code>
    <div v-if="preview.colorFormats" class="color-actions">
      <button
        v-for="format in ['hex', 'rgb'] as const"
        :key="format"
        type="button"
        :disabled="copying"
        @click.stop="copyColor(format)"
        @keydown.enter.stop
        @keydown.space.stop
      >
        复制为 {{ format.toUpperCase() }}
      </button>
    </div>
    <span class="color-copy-status" role="status">{{
      copiedFormat ? `已复制 ${copiedFormat}` : ''
    }}</span>
  </template>
</template>

<style scoped>
.color-swatch {
  flex: 1;
  min-height: 48px;
  border: 1px solid var(--color-border-light);
  border-radius: 10px;
  overflow: hidden;
}
.color-swatch > div {
  height: 100%;
}
.color-value {
  font-family: var(--font-family-mono, monospace);
  font-size: 18px;
  text-align: center;
  overflow-wrap: anywhere;
}
.color-rgb {
  text-align: center;
  color: var(--color-text-secondary);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.color-value,
.color-rgb,
.color-actions,
.color-copy-status {
  flex-shrink: 0;
}
.color-actions {
  display: flex;
  justify-content: center;
  gap: 8px;
}
.color-actions button {
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-secondary);
  color: var(--color-text-primary);
  padding: 4px 8px;
  font-size: 11px;
  cursor: pointer;
}
.color-actions button:hover {
  background: var(--color-selection);
  color: var(--color-primary);
}
.color-actions button:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}
.color-copy-status {
  min-height: 16px;
  text-align: center;
  font-size: 11px;
  line-height: 16px;
  color: var(--color-text-secondary);
}
.color-swatch.expanded {
  flex: none;
  height: 220px;
}
</style>
