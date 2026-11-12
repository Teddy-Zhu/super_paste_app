<script setup lang="ts">
import { ref, onMounted } from 'vue'
import IconTag from '~icons/mdi/tag-outline'
import IconPlus from '~icons/mdi/plus'
import IconEdit from '~icons/mdi/pencil-outline'
import IconDelete from '~icons/mdi/trash-can-outline'
import TagEditorDialog from './TagEditorDialog.vue'
import { SETTINGS_LABELS } from '@renderer/constants/desktop-ui'
import { tags } from '@renderer/api/tags'
import { confirmAction } from '@renderer/api/dialog'
import type { TagGroup } from '@renderer/api/types'

const allTags = ref<TagGroup[]>([])
const loading = ref(true)
const error = ref('')
const deletingId = ref<string | null>(null)
// undefined closes the editor; null creates a tag; a record edits that tag.
const editorTag = ref<TagGroup | null>()

const loadAllTags = async (): Promise<void> => {
  loading.value = true
  error.value = ''
  try {
    allTags.value = await tags.getAllTags()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
  }
}
const deleteTag = async (tag: TagGroup): Promise<void> => {
  if (deletingId.value) return
  deletingId.value = tag.id
  try {
    if (!(await confirmAction(`删除标签“${tag.name}”？相关剪贴板内容会保留，标签绑定将被移除。`)))
      return
    await tags.deleteTag(tag.id)
    allTags.value = allTags.value.filter((item) => item.id !== tag.id)
    await loadAllTags()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    deletingId.value = null
  }
}
const formatDate = (timestamp: number): string => new Date(timestamp).toLocaleString('zh-CN')
onMounted(() => void loadAllTags())
</script>

<template>
  <section class="content-section tags-management" :aria-busy="loading">
    <header class="section-header">
      <h2 class="section-title">标签管理</h2>
      <p class="section-subtitle">整理剪贴板内容的分类标签</p>
    </header>
    <div class="tags-toolbar">
      <span class="tags-count">{{ allTags.length }} 个标签</span>
      <button
        type="button"
        class="create-tag-button"
        :disabled="loading || !!deletingId"
        @click="editorTag = null"
      >
        <IconPlus />新建标签
      </button>
    </div>
    <div v-if="error" class="tags-error" role="alert">
      <span>{{ error }}</span>
      <button
        type="button"
        class="form-button secondary"
        :disabled="loading || !!deletingId"
        @click="loadAllTags"
      >
        重新加载
      </button>
    </div>
    <div v-if="loading && !allTags.length" class="loading-state" role="status">
      <div class="loading-spinner" />
      <span>正在加载标签…</span>
    </div>
    <div v-else-if="!error && !allTags.length" class="empty-state">
      <div class="empty-icon" aria-hidden="true"><IconTag /></div>
      <p class="empty-text">暂无标签</p>
      <button type="button" class="form-button secondary" @click="editorTag = null">
        新建标签
      </button>
    </div>
    <div v-else-if="allTags.length" class="tags-grid">
      <div
        v-for="tag in allTags"
        :key="tag.id"
        class="tag-card"
        :class="{ 'tag-card--busy': deletingId === tag.id }"
        @dblclick="!deletingId && (editorTag = tag)"
      >
        <span class="tag-color-dot" :style="{ backgroundColor: tag.color }" aria-hidden="true" />
        <div
          class="tag-info"
          :title="`创建于 ${formatDate(tag.createdAt)}\n更新于 ${formatDate(tag.updatedAt)}`"
        >
          <h3 class="tag-name">{{ tag.name }}</h3>
          <p v-if="tag.description" class="tag-description">{{ tag.description }}</p>
        </div>
        <div class="tag-actions-menu">
          <button
            type="button"
            class="tag-action-button"
            :disabled="!!deletingId"
            :aria-label="`${SETTINGS_LABELS.editTag}：${tag.name}`"
            :title="SETTINGS_LABELS.editTag"
            @click="editorTag = tag"
            @dblclick.stop
          >
            <IconEdit />
          </button>
          <button
            type="button"
            class="tag-action-button danger"
            :disabled="!!deletingId"
            :aria-label="`${SETTINGS_LABELS.deleteTag}：${tag.name}`"
            :title="SETTINGS_LABELS.deleteTag"
            @click="deleteTag(tag)"
            @dblclick.stop
          >
            <IconDelete />
          </button>
        </div>
      </div>
    </div>
    <TagEditorDialog
      v-if="editorTag !== undefined"
      :tag="editorTag"
      @close="editorTag = undefined"
      @saved="loadAllTags"
    />
  </section>
</template>
