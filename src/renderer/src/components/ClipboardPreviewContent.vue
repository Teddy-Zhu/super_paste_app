<script setup lang="ts">
import { computed } from 'vue'
import type { PreviewProps, PreviewEvents } from '@renderer/features/clipboard/preview/types'
import { previewComponents } from '@renderer/features/clipboard/preview/components'

const props = withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
const emit = defineEmits<PreviewEvents>()
const renderer = computed(() =>
  props.plain && ['rich', 'table'].includes(props.preview.kind)
    ? previewComponents.text
    : previewComponents[props.preview.kind]
)
</script>

<template>
  <div class="preview-content" :class="[{ expanded, 'plain-mode': plain }, `kind-${preview.kind}`]">
    <p v-if="preview.notice" class="preview-notice">{{ preview.notice }}</p>
    <component
      :is="renderer"
      :preview="preview"
      :expanded="expanded"
      :plain="plain"
      @image-load="emit('imageLoad', $event)"
    />
  </div>
</template>

<style scoped>
.preview-content {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  color: var(--color-text-primary);
  font-size: 14px;
  line-height: 1.65;
}
:deep(.preview-notice) {
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 11px;
  flex-shrink: 0;
}
:deep(.checkerboard) {
  background-color: var(--color-bg-primary);
  background-image: conic-gradient(
    var(--color-border-light) 25%,
    transparent 0 50%,
    var(--color-border-light) 0 75%,
    transparent 0
  );
  background-size: 16px 16px;
}
.preview-content.expanded {
  height: auto;
  min-height: 100%;
  overflow: visible;
  gap: 16px;
}
</style>
