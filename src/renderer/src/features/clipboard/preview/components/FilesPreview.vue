<script setup lang="ts">
import { computed } from 'vue'
import { PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import IconFile from '~icons/mdi/file-outline'
import type { PreviewProps, PreviewEvents } from '../types'

const props = withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
const visibleFiles = computed(() =>
  props.expanded ? props.preview.files : props.preview.files.slice(0, PREVIEW_LIMITS.fileRows)
)
</script>

<template>
  <div class="file-list" :class="{ expanded }">
    <div v-for="file in visibleFiles" :key="file.path" class="file-row" :title="file.path">
      <div class="file-symbol">
        <IconFile /><span>{{ file.extension }}</span>
      </div>
      <div class="file-info">
        <div class="file-name" :class="{ expanded }" dir="auto">{{ file.name }}</div>
        <div v-if="file.directory" class="file-directory" :class="{ expanded }" dir="auto">
          {{ file.directory }}
        </div>
      </div>
    </div>
  </div>
  <p v-if="visibleFiles.length < preview.files.length" class="preview-notice">
    +{{ preview.files.length - visibleFiles.length }} 个文件
  </p>
</template>

<style scoped>
.file-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  overflow: hidden;
}
.file-row {
  display: flex;
  gap: 10px;
  align-items: center;
  min-width: 0;
  flex-shrink: 0;
  padding: 8px 0;
}
.file-symbol {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
  color: var(--color-type-teal);
  width: 40px;
}
.file-symbol svg {
  width: 28px;
  height: 28px;
}
.file-symbol span {
  font-size: 9px;
  line-height: 1.3;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-info {
  min-width: 0;
}
.file-name {
  font-size: 14px;
  font-weight: 600;
  overflow-wrap: anywhere;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.file-directory {
  font-size: 11px;
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-list.expanded {
  overflow: visible;
}
.file-name.expanded {
  display: block;
}
.file-directory.expanded {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
