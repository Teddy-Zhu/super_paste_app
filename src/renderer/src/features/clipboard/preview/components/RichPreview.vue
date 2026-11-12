<script setup lang="ts">
import type { PreviewProps, PreviewEvents } from '../types'

withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
</script>

<template>
  <div class="rich-content" :class="{ expanded }" v-html="preview.html"></div>
</template>

<style scoped>
.rich-content {
  min-height: 0;
  overflow: hidden;
  overflow-wrap: anywhere;
  isolation: isolate;
  /* 富文本可能自带文字和底色，保留原稿颜色并提供稳定的纸张背景。 */
  --color-text-secondary: #64748b;
  --color-control-border: #cbd5e1;
  color-scheme: light;
  color: #334155;
  background: #fff;
}
.rich-content:not(.expanded) {
  pointer-events: none;
}
.rich-content :deep(*) {
  box-sizing: border-box;
  max-width: 100%;
}
.rich-content :deep(> :first-child) {
  margin-top: 0;
}
.rich-content :deep(p),
.rich-content :deep(blockquote),
.rich-content :deep(ul),
.rich-content :deep(ol) {
  margin-block: 0.4em;
}
.rich-content :deep(ul),
.rich-content :deep(ol) {
  padding-left: 1.5em;
}
.rich-content :deep(h1),
.rich-content :deep(h2),
.rich-content :deep(h3) {
  font-size: 1.15em;
  margin-block: 0.5em;
}
.rich-content :deep(pre),
.rich-content :deep(code) {
  font-family: var(--font-family-mono, monospace);
  white-space: pre-wrap;
}
.rich-content :deep(img) {
  height: auto;
  object-fit: contain;
}
.rich-content :deep(table) {
  border-collapse: collapse;
  font-size: 12px;
  text-align: start;
}
.rich-content :deep(table) {
  table-layout: fixed;
  width: 100%;
}
.rich-content :deep(td),
.rich-content :deep(th) {
  border: 1px solid var(--color-control-border);
  padding: 5px 8px;
  vertical-align: top;
}
.rich-content :deep(blockquote) {
  border-left: 3px solid var(--color-control-border);
  padding-left: 10px;
  color: var(--color-text-secondary);
}
.rich-content.expanded {
  overflow: visible;
}
</style>
