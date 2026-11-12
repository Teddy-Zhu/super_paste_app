<script setup lang="ts">
import { PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import { previewExcerpt } from '../providers/text'
import IconLink from '~icons/mdi/link-variant'
import type { PreviewProps, PreviewEvents } from '../types'

withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
</script>

<template>
  <template v-if="preview.url">
    <div class="identity-heading">
      <IconLink /><strong>{{ preview.url.host }}</strong>
    </div>
    <p class="url-path" dir="auto">
      {{ expanded ? preview.url.detail : previewExcerpt(preview.url.detail, PREVIEW_LIMITS.text) }}
    </p>
    <p class="original-url" dir="auto">
      {{ expanded ? preview.url.href : previewExcerpt(preview.url.href, PREVIEW_LIMITS.text) }}
    </p>
  </template>
</template>

<style scoped>
.identity-heading {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  color: var(--color-type-orange);
  font-size: 18px;
  overflow-wrap: anywhere;
}
.identity-heading svg {
  flex-shrink: 0;
  margin-top: 5px;
}
.url-path {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 14px;
}
.original-url {
  margin: auto 0 0;
  font-size: 11px;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}
</style>
