<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { AppInfo } from '@renderer/api/types'
import type { ClipboardPreview } from '@renderer/utils/clipboard-preview'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'
import IconClose from '~icons/mdi/close'
import ClipboardPreviewContent from './ClipboardPreviewContent.vue'
import ClipboardSourceApp from './ClipboardSourceApp.vue'

const props = defineProps<{ preview: ClipboardPreview; source?: AppInfo; summary?: string }>()
const emit = defineEmits<{ close: [] }>()
const dialog = ref<HTMLDialogElement>()
const plain = ref(false)
const canToggle = computed(
  () =>
    Boolean(props.preview.text) &&
    (['rich', 'table'].includes(props.preview.kind) ||
      props.preview.text !== props.preview.displayText)
)

onMounted(() => dialog.value?.showModal())
onBeforeUnmount(() => dialog.value?.close())
</script>

<template>
  <Teleport to="body">
    <dialog
      ref="dialog"
      class="clipboard-preview-dialog"
      :aria-label="`${preview.label} · ${PREVIEW_LABELS.details}`"
      @cancel.prevent="emit('close')"
      @click.stop="$event.target === $event.currentTarget && emit('close')"
      @keydown.stop
      @keyup.stop
      @wheel.stop
      @contextmenu.stop
    >
      <div class="dialog-layout">
        <header class="dialog-header">
          <div class="dialog-title">
            <strong>{{ preview.label }}</strong
            ><span>{{ summary || preview.summary }}</span>
          </div>
          <button
            v-if="canToggle"
            type="button"
            class="preview-toggle"
            :aria-pressed="plain"
            @click="plain = !plain"
          >
            {{ plain ? PREVIEW_LABELS.formatted : PREVIEW_LABELS.original }}
          </button>
          <ClipboardSourceApp v-if="source" :app="source" />
          <button
            type="button"
            class="close-button"
            :aria-label="PREVIEW_LABELS.close"
            @click="emit('close')"
          >
            <IconClose />
          </button>
        </header>
        <div class="dialog-body">
          <ClipboardPreviewContent :preview="preview" :plain="plain" expanded />
        </div>
      </div>
    </dialog>
  </Teleport>
</template>

<style scoped>
.clipboard-preview-dialog {
  width: min(860px, calc(100vw - 24px));
  max-width: calc(100vw - 24px);
  height: min(720px, calc(100dvh - 24px));
  max-height: calc(100dvh - 24px);
  margin: auto;
  padding: 0;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-lg);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-soft-xl);
  overflow: hidden;
}
.clipboard-preview-dialog[open] {
  animation: surface-appear var(--duration-normal) var(--ease-out);
}
.clipboard-preview-dialog::backdrop {
  background: var(--color-backdrop);
}
.dialog-layout {
  display: flex;
  flex-direction: column;
  height: 100%;
}
.dialog-header {
  display: flex;
  gap: 12px;
  align-items: center;
  padding: 10px 12px;
  background: var(--color-titlebar);
  border-bottom: 1px solid var(--color-border-light);
  flex-shrink: 0;
}
.dialog-title {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
  gap: 3px;
}
.dialog-title strong {
  font-size: 13px;
  font-weight: 600;
}
.dialog-title span {
  font-size: 11px;
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dialog-header button {
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-secondary);
  cursor: pointer;
  flex-shrink: 0;
}
.preview-toggle[aria-pressed='true'] {
  background: var(--color-selection);
  border-color: var(--color-primary);
  color: var(--color-primary);
}
.preview-toggle {
  font-size: 12px;
  padding: 5px 9px;
}
.close-button {
  display: grid;
  place-items: center;
  padding: 6px;
  font-size: 18px;
}
.dialog-header button:hover {
  background: var(--color-control-hover);
}
.dialog-header button:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}
.dialog-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 16px;
  overscroll-behavior: contain;
  user-select: text;
}
</style>
