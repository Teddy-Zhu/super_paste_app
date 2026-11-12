<script setup lang="ts">
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'
import { previewExcerpt } from '../providers/text'
import IconFile from '~icons/mdi/file-outline'
import ImageDisplay from '@renderer/components/ImageDisplay.vue'
import type { PreviewProps, PreviewEvents } from '../types'

withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
const emit = defineEmits<PreviewEvents>()
</script>

<template>
  <template v-if="preview.image">
    <div class="image-frame checkerboard">
      <ImageDisplay
        :image-data="preview.image"
        :alt="PREVIEW_LABELS.image"
        image-class="preview-image"
        @load="emit('imageLoad', $event)"
      />
    </div>
    <div v-if="preview.text.trim()" class="image-caption" :class="{ expanded }">
      <span class="caption-label">{{ PREVIEW_LABELS.caption }}</span>
      <p dir="auto">
        {{ expanded ? preview.text : previewExcerpt(preview.text, 300) }}
      </p>
    </div>
    <div v-if="preview.files.length" class="image-filename" :title="preview.files[0].path">
      <IconFile />{{ preview.files[0].name }}
    </div>
  </template>
</template>

<style scoped>
.image-frame {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.image-frame :deep(.preview-image) {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.image-caption {
  flex-shrink: 0;
  min-width: 0;
  border-top: 1px solid var(--color-border-light);
  padding-top: 6px;
  max-height: 35%;
  overflow: hidden;
}
.image-caption p {
  margin: 2px 0 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.image-filename {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text-secondary);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
}
.image-filename svg {
  flex-shrink: 0;
}
.image-frame.expanded {
  flex: none;
  min-height: 100px;
  overflow: visible;
}
.image-frame.expanded :deep(.preview-image) {
  width: auto;
  height: auto;
  max-width: 100%;
}
.image-caption.expanded {
  max-height: none;
  overflow: visible;
}
.image-caption.expanded p {
  display: block;
}
.caption-label {
  color: var(--color-text-secondary);
  font-size: 10px;
  font-weight: 600;
}
</style>
