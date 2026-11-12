<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, reactive, ref } from 'vue'
import IconTag from '~icons/mdi/tag-outline'
import ColorPicker from '@renderer/components/ColorPicker.vue'
import { FORM_CONTROL_TEXT } from '@renderer/constants/form-controls'
import { tags } from '@renderer/api/tags'
import type { TagGroup } from '@renderer/api/types'

const props = defineProps<{ tag: TagGroup | null }>()
const emit = defineEmits<{ saved: []; close: [] }>()
const dialog = ref<HTMLDialogElement>()
const nameInput = ref<HTMLInputElement>()
const saving = ref(false)
const error = ref('')
const invalidName = ref(false)
const editing = computed(() => props.tag !== null)
const form = reactive({
  name: props.tag?.name || '',
  color: props.tag?.color || '#3b82f6',
  description: props.tag?.description || ''
})
const presetColors = [
  '#3b82f6',
  '#10b981',
  '#8b5cf6',
  '#f59e0b',
  '#ef4444',
  '#06b6d4',
  '#84cc16',
  '#f97316',
  '#ec4899',
  '#6366f1'
]

const close = (): void => {
  if (saving.value) return
  dialog.value?.close()
  emit('close')
}
const save = async (): Promise<void> => {
  if (saving.value) return
  invalidName.value = !form.name.trim()
  if (invalidName.value) {
    error.value = '请输入标签名称'
    nameInput.value?.focus()
    return
  }
  saving.value = true
  error.value = ''
  try {
    const values = {
      name: form.name.trim(),
      color: form.color,
      description: form.description.trim()
    }
    if (props.tag) await tags.updateTag(props.tag.id, values)
    else await tags.createTag(values.name, values.color, values.description || undefined)
    emit('saved')
    saving.value = false
    close()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    saving.value = false
  }
}
onMounted(() => {
  dialog.value?.showModal()
  nameInput.value?.focus()
  if (editing.value) nameInput.value?.select()
})
onBeforeUnmount(() => dialog.value?.close())
</script>

<template>
  <dialog
    ref="dialog"
    class="tag-editor"
    aria-labelledby="tag-editor-title"
    @cancel.prevent="close"
  >
    <form
      novalidate
      :aria-busy="saving"
      @submit.prevent="save"
      @keydown.ctrl.enter.prevent="save"
      @keydown.meta.enter.prevent="save"
    >
      <header class="tag-editor-header">
        <span class="tag-editor-symbol" :style="{ color: form.color }"><IconTag /></span>
        <h3 id="tag-editor-title">{{ editing ? '编辑标签' : '新建标签' }}</h3>
      </header>
      <fieldset class="tag-editor-fields" :disabled="saving">
        <label class="tag-editor-field" for="tag-name">
          <span>名称</span>
          <input
            id="tag-name"
            ref="nameInput"
            v-model="form.name"
            type="text"
            maxlength="20"
            autocomplete="off"
            placeholder="标签名称"
            :aria-invalid="invalidName || undefined"
            :aria-describedby="error ? 'tag-editor-error' : undefined"
            @input="invalidName = false"
          />
        </label>
        <div class="tag-editor-field">
          <span id="tag-color-label">颜色</span>
          <div class="tag-editor-colors" role="group" aria-labelledby="tag-color-label">
            <button
              v-for="color in presetColors"
              :key="color"
              type="button"
              class="tag-color-option"
              :class="{ selected: form.color.toLowerCase() === color }"
              :style="{ backgroundColor: color }"
              :aria-label="color"
              :aria-pressed="form.color.toLowerCase() === color"
              @click="form.color = color"
            />
            <ColorPicker
              v-model="form.color"
              :disabled="saving"
              :aria-label="FORM_CONTROL_TEXT.customColor"
            />
          </div>
        </div>
        <label class="tag-editor-field" for="tag-description">
          <span>描述 <small>可选</small></span>
          <textarea
            id="tag-description"
            v-model="form.description"
            maxlength="100"
            rows="2"
            placeholder="添加简短描述"
          />
        </label>
        <p v-if="error" id="tag-editor-error" class="tag-editor-error" role="alert">{{ error }}</p>
      </fieldset>
      <footer class="tag-editor-footer">
        <button type="button" class="form-button secondary" :disabled="saving" @click="close">
          取消
        </button>
        <button type="submit" class="form-button primary" :disabled="saving">
          {{ saving ? '正在保存…' : editing ? '保存' : '创建' }}
        </button>
      </footer>
    </form>
  </dialog>
</template>

<style scoped>
.tag-editor {
  width: min(420px, calc(100vw - 40px));
  max-height: calc(100vh - 40px);
  margin: auto;
  padding: 0;
  border: 1px solid var(--color-border-strong);
  border-radius: var(--radius-lg);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-soft-xl);
}
.tag-editor::backdrop {
  background: var(--color-backdrop);
}
.tag-editor-header {
  display: flex;
  gap: 10px;
  align-items: center;
  padding: 20px 22px 16px;
}
.tag-editor-header h3 {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}
.tag-editor-symbol {
  display: grid;
  place-items: center;
  font-size: 22px;
}
.tag-editor-fields {
  display: flex;
  flex-direction: column;
  gap: 18px;
  min-width: 0;
  margin: 0;
  padding: 0 22px 22px;
  border: 0;
}
.tag-editor-field {
  display: flex;
  flex-direction: column;
  gap: 7px;
  font-size: 12px;
  font-weight: 500;
}
.tag-editor-field small {
  margin-left: 4px;
  font-size: 11px;
  font-weight: 400;
  color: var(--color-text-tertiary);
}
.tag-editor-field input,
.tag-editor-field textarea {
  width: 100%;
  padding: 7px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  font-size: 13px;
}
.tag-editor-field input[aria-invalid='true'] {
  border-color: var(--color-danger);
}
.tag-editor-field textarea {
  resize: none;
}
.tag-editor-colors {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  align-items: center;
}
.tag-color-option {
  width: 22px;
  height: 22px;
  padding: 0;
  border: 2px solid transparent;
  border-radius: 50%;
  cursor: pointer;
  flex-shrink: 0;
}
.tag-color-option.selected {
  border-color: var(--color-bg-primary);
  box-shadow: 0 0 0 2px var(--color-primary);
}
.tag-editor-error {
  margin: 0;
  color: var(--color-danger);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.tag-editor-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 22px;
  background: var(--color-bg-secondary);
  border-top: 1px solid var(--color-border-light);
}
.tag-editor-footer button {
  min-width: 70px;
}
.tag-editor button:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}
</style>
