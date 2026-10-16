<script setup lang="ts">
import { tags } from '@renderer/api/renderer-api'
import type { TagGroup } from '@renderer/api/types'

// 标签管理相关状态
const allTags = ref<TagGroup[]>([])
const isLoadingTags = ref(false)
const isCreatingTag = ref(false)
const isEditingTag = ref(false)
const editingTagId = ref<string | null>(null)

// 新建/编辑标签表单
const tagForm = ref({
  name: '',
  color: '#3b82f6',
  description: ''
})

// 预设颜色选项
const presetColors = [
  '#3b82f6', // 蓝色
  '#10b981', // 绿色
  '#8b5cf6', // 紫色
  '#f59e0b', // 橙色
  '#ef4444', // 红色
  '#06b6d4', // 青色
  '#84cc16', // 柠檬绿
  '#f97316', // 深橙色
  '#ec4899', // 粉色
  '#6366f1' // 靛蓝色
]

// 加载所有标签
const loadAllTags = async () => {
  try {
    isLoadingTags.value = true
    allTags.value = await tags.getAllTags()
  } catch (error) {
    console.error('加载标签失败:', error)
    // 可以在这里添加错误提示
  } finally {
    isLoadingTags.value = false
  }
}

// 重置标签表单
const resetTagForm = () => {
  tagForm.value = {
    name: '',
    color: '#3b82f6',
    description: ''
  }
  isCreatingTag.value = false
  isEditingTag.value = false
  editingTagId.value = null
}

// 开始创建新标签
const startCreateTag = () => {
  resetTagForm()
  isCreatingTag.value = true
}

// 开始编辑标签
const startEditTag = (tag: TagGroup) => {
  tagForm.value = {
    name: tag.name,
    color: tag.color,
    description: tag.description || ''
  }
  editingTagId.value = tag.id
  isEditingTag.value = true
}

// 保存标签（创建或更新）
const saveTag = async () => {
  if (!tagForm.value.name.trim()) {
    alert('请输入标签名称')
    return
  }

  try {
    if (isCreatingTag.value) {
      // 创建新标签
      await tags.createTag(
        tagForm.value.name.trim(),
        tagForm.value.color,
        tagForm.value.description.trim() || undefined
      )
    } else if (isEditingTag.value && editingTagId.value) {
      // 更新标签
      await tags.updateTag(editingTagId.value, {
        name: tagForm.value.name.trim(),
        color: tagForm.value.color,
        description: tagForm.value.description.trim() || undefined
      })
    }

    // 重新加载标签列表
    await loadAllTags()
    resetTagForm()
  } catch (error) {
    console.error('保存标签失败:', error)
    alert('保存标签失败: ' + (error as Error).message)
  }
}

// 删除标签
const deleteTag = async (tag: TagGroup) => {
  if (!confirm(`确定要删除标签"${tag.name}"吗？这将同时移除所有相关的标签绑定。`)) {
    return
  }

  try {
    await tags.deleteTag(tag.id)
    await loadAllTags()
  } catch (error) {
    console.error('删除标签失败:', error)
    alert('删除标签失败: ' + (error as Error).message)
  }
}

// 取消编辑
const cancelEdit = () => {
  resetTagForm()
}

// 格式化时间显示
const formatDate = (timestamp: number): string => {
  return new Date(timestamp).toLocaleString('zh-CN')
}

// 组件挂载时加载标签
onMounted(() => {
  loadAllTags()
})
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">标签管理</h3>
      <p class="section-subtitle">创建和管理剪贴板内容的标签分类</p>
    </div>

    <!-- 创建新标签按钮 -->
    <div class="tag-actions">
      <button
        class="create-tag-button"
        @click="startCreateTag"
        :disabled="isCreatingTag || isEditingTag"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <line x1="12" y1="5" x2="12" y2="19"></line>
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
        新建标签
      </button>
    </div>

    <!-- 标签表单 -->
    <div v-if="isCreatingTag || isEditingTag" class="tag-form-card">
      <div class="tag-form-header">
        <h4 class="tag-form-title">{{ isCreatingTag ? '创建新标签' : '编辑标签' }}</h4>
      </div>

      <div class="tag-form-content">
        <!-- 标签名称 -->
        <div class="form-group">
          <label class="form-label">标签名称</label>
          <input
            type="text"
            v-model="tagForm.name"
            class="form-input"
            placeholder="请输入标签名称"
            maxlength="20"
          />
        </div>

        <!-- 标签颜色 -->
        <div class="form-group">
          <label class="form-label">标签颜色</label>
          <div class="color-picker">
            <div class="color-options">
              <div
                v-for="color in presetColors"
                :key="color"
                class="color-option"
                :class="{ active: tagForm.color === color }"
                :style="{ backgroundColor: color }"
                @click="tagForm.color = color"
              ></div>
            </div>
            <input type="color" v-model="tagForm.color" class="color-input" />
          </div>
        </div>

        <!-- 标签描述 -->
        <div class="form-group">
          <label class="form-label">描述（可选）</label>
          <textarea
            v-model="tagForm.description"
            class="form-textarea"
            placeholder="请输入标签描述"
            maxlength="100"
            rows="3"
          ></textarea>
        </div>

        <!-- 表单按钮 -->
        <div class="form-actions">
          <button class="form-button primary" @click="saveTag">
            {{ isCreatingTag ? '创建' : '保存' }}
          </button>
          <button class="form-button secondary" @click="cancelEdit">取消</button>
        </div>
      </div>
    </div>

    <!-- 标签列表 -->
    <div class="tags-list">
      <div v-if="isLoadingTags" class="loading-state">
        <div class="loading-spinner"></div>
        <span>加载标签中...</span>
      </div>

      <div v-else-if="allTags.length === 0" class="empty-state">
        <div class="empty-icon">🏷️</div>
        <p class="empty-text">暂无标签</p>
        <p class="empty-hint">点击"新建标签"创建第一个标签</p>
      </div>

      <div v-else class="tags-grid">
        <div v-for="tag in allTags" :key="tag.id" class="tag-card">
          <div class="tag-card-header">
            <div class="tag-info">
              <div class="tag-color-dot" :style="{ backgroundColor: tag.color }"></div>
              <h4 class="tag-name">{{ tag.name }}</h4>
            </div>
            <div class="tag-actions-menu">
              <button class="tag-action-button" @click="startEditTag(tag)">
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
                  <path d="m18.5 2.5 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
                </svg>
              </button>
              <button class="tag-action-button danger" @click="deleteTag(tag)">
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <polyline points="3,6 5,6 21,6"></polyline>
                  <path
                    d="m19,6v14a2,2 0 0,1 -2,2H7a2,2 0 0,1 -2,-2V6m3,0V4a2,2 0 0,1 2,-2h4a2,2 0 0,1 2,2v2"
                  ></path>
                </svg>
              </button>
            </div>
          </div>

          <div v-if="tag.description" class="tag-description">
            {{ tag.description }}
          </div>

          <div class="tag-meta">
            <span class="tag-date">创建于 {{ formatDate(tag.createdAt) }}</span>
            <span v-if="tag.updatedAt !== tag.createdAt" class="tag-date">
              更新于 {{ formatDate(tag.updatedAt) }}
            </span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
