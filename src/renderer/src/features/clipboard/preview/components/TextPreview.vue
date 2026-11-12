<script setup lang="ts">
import { computed } from 'vue'
import { PREVIEW_LABELS, PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import { previewExcerpt } from '../providers/text'
import type { PreviewProps, PreviewEvents } from '../types'

const props = withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
const text = computed(() => (props.plain ? props.preview.text : props.preview.displayText))
const displayedText = computed(() => {
  const value = props.expanded ? text.value : previewExcerpt(text.value, PREVIEW_LIMITS.text)
  return value && !value.trim()
    ? value
        .replace(/ /g, '·')
        .replace(/\t/g, '→   ')
        .replace(/\r\n|\r|\n/g, '↵\n')
    : value
})
const compactText = computed(() => text.value.length <= 80 && !/[\r\n]/.test(text.value))
</script>

<template>
  <span v-if="!text.trim()" class="preview-notice">{{ PREVIEW_LABELS.whitespace }}</span>
  <pre
    class="text-preview"
    :class="{ expanded, 'code-preview': preview.kind === 'code', 'compact-text': compactText }"
    dir="auto"
    >{{ displayedText }}</pre>
  <span v-if="!expanded && text.length > PREVIEW_LIMITS.text" class="preview-notice">{{
    PREVIEW_LABELS.previewExcerpt
  }}</span>
</template>

<style scoped>
.text-preview {
  margin: 0;
  min-height: 0;
  overflow: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: inherit;
  tab-size: 4;
}
.compact-text {
  font-size: 22px;
  line-height: 1.5;
  font-weight: 500;
  margin-block: auto;
}
.code-preview {
  font-family: var(--font-family-mono, monospace);
  font-size: 12px;
  line-height: 1.7;
  padding: 10px;
  background: var(--color-bg-secondary);
  border-radius: 8px;
}
.text-preview.expanded {
  overflow: visible;
  font-size: 14px;
  margin: 0;
}
.code-preview.expanded {
  white-space: pre;
  min-width: max-content;
}
</style>
