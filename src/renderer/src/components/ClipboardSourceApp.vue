<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { AppInfo } from '@renderer/api/types'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'

const props = defineProps<{ app: AppInfo }>()
const iconFailed = ref(false)
const hasIcon = computed(() => Boolean(props.app.icon) && !iconFailed.value)

watch(
  () => props.app.icon,
  () => {
    iconFailed.value = false
  }
)
</script>

<template>
  <div
    class="source-app"
    :class="{ 'source-app--with-icon': hasIcon }"
    :title="`${PREVIEW_LABELS.source} · ${app.application}`"
  >
    <img v-if="hasIcon" :src="app.icon" alt="" @error="iconFailed = true" />
    <span class="source-name">{{ app.application }}</span>
  </div>
</template>

<style scoped>
.source-app {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  min-width: 0;
  max-width: 120px;
  flex-shrink: 0;
}
.source-app img {
  width: 32px;
  height: 32px;
  object-fit: contain;
}
.source-name {
  max-width: 100%;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  color: var(--color-text-secondary);
  font-size: 12px;
  line-height: 16px;
}
.source-app--with-icon .source-name {
  font-size: 10px;
  line-height: 14px;
}
</style>
