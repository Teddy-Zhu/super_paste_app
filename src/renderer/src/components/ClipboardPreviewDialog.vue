<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { ClipboardPreview } from '@renderer/utils/clipboard-preview'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'
import IconClose from '~icons/mdi/close'
import ClipboardPreviewContent from './ClipboardPreviewContent.vue'

const props = defineProps<{ preview: ClipboardPreview; source?: string; summary?: string }>()
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
            ><span>{{ [summary || preview.summary, source].filter(Boolean).join(' · ') }}</span>
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
  border: 1px solid #cbd5e1;
  border-radius: 14px;
  background: #fff;
  color: #334155;
  box-shadow: 0 16px 60px #0f172a40;
  overflow: hidden;
}
.clipboard-preview-dialog::backdrop {
  background: #0f172a66;
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
  padding: 12px 16px;
  border-bottom: 1px solid #e2e8f0;
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
  font-size: 15px;
}
.dialog-title span {
  font-size: 11px;
  color: #64748b;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dialog-header button {
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  background: #f8fafc;
  color: #475569;
  cursor: pointer;
  flex-shrink: 0;
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
  background: #e2e8f0;
}
.dialog-header button:focus-visible {
  outline: 2px solid #3b82f6;
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
