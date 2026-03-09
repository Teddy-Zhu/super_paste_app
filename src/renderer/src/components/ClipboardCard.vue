<script setup lang="ts">
import type { ClipboardItem, TagGroup, AppInfo } from '@renderer/api/types'
import { computed, ref } from 'vue'
import { convertRtfToHtml } from 'rtf-to-html-converter'
import DOMPurify from 'dompurify'
import IconFile from '~icons/mdi/file'
import IconFileMultiple from '~icons/mdi/file-multiple'
import IconTextBox from '~icons/mdi/text-box'
import IconTag from '~icons/mdi/tag'
import IconCheck from '~icons/mdi/check'
import IconDelete from '~icons/mdi/delete'
import IconCircle from '~icons/mdi/circle'
import { useContextMenu, type ContextMenuItem } from '@renderer/composables/useContextMenu'
import { tags, clipboard } from '@renderer/api/renderer-api'
import ImageDisplay from './ImageDisplay.vue'

interface Props {
  item: ClipboardItem
  index: number
  isSelected: boolean
  isQuickSelectMode: boolean
  allTags?: TagGroup[]
  activeTagId?: string
  apps?: Record<string, AppInfo>
}

const props = defineProps<Props>()

const emit = defineEmits<{
  click: [itemId: string]
  restoreAsText: [itemId: string]
  restoreOcrText: [itemId: string]
  parseOcr: [itemId: string]
  parseQRCode: [itemId: string]
  generateQRCode: [itemId: string]
  delete: [itemId: string]
}>()

const PREVIEW_MAX_LENGTH = 150
const COLOR_REGEX = /^#[0-9a-fA-F]{6}$|^#[0-9a-fA-F]{8}$/

const TYPE_LABEL_MAP: Record<string, string> = {
  text: 'Text',
  image: 'Image',
  html: 'HTML',
  url: 'URL',
  files: 'Files',
  email: 'Email',
  phone: 'Phone',
  code: 'Code',
  rtf: 'RTF',
  color: 'Color',
  empty: 'Empty'
}

const CARD_THEME_MAP: Record<string, string> = {
  text: 'blue',
  image: 'green',
  html: 'purple',
  url: 'orange',
  files: 'teal',
  email: 'red',
  phone: 'red',
  code: 'purple',
  rtf: 'blue',
  color: 'green',
  empty: 'gray'
}

const stripHtmlTags = (html: string): string => html.replace(/<[^>]*>/g, '')

const truncate = (text: string, maxLength: number): string => {
  return text.length > maxLength ? `${text.slice(0, maxLength)}...` : text
}

const getFileNameFromPath = (path: string): string => {
  return path.split('/').pop() || 'Unknown file'
}

const safeDecodeURIComponent = (value: string): string => {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

const hasConvertibleContent = (item: ClipboardItem): boolean => {
  const { text, html, rtf, files } = item.content
  return Boolean(text?.trim() || html?.trim() || rtf?.trim() || files?.length)
}

const sourceAppInfo = computed(() => {
  return props.item.source_app && props.apps ? props.apps[props.item.source_app] || null : null
})

const rtfToHtml = computed(() => {
  if (!props.item.content.rtf) return ''

  try {
    return DOMPurify.sanitize(convertRtfToHtml(props.item.content.rtf) || '<p>富文本内容</p>')
  } catch (error) {
    console.warn('RTF 转换失败:', error)
    return DOMPurify.sanitize('<p>富文本内容</p>')
  }
})

const htmlContent = computed(() => {
  return props.item.content.html ? DOMPurify.sanitize(props.item.content.html) : ''
})

const getContentPreview = (item: ClipboardItem): string => {
  const content = item.content

  if (content.text) {
    return truncate(content.text, PREVIEW_MAX_LENGTH)
  }

  if (content.html) {
    return truncate(stripHtmlTags(content.html), PREVIEW_MAX_LENGTH)
  }

  if (content.image) {
    return 'Image content'
  }

  if (content.rtf) {
    return 'Rich text content'
  }

  if (content.files && content.files.length > 0) {
    const fileCount = content.files.length
    const fileName = getFileNameFromPath(content.files[0])
    return fileCount === 1 ? `File: ${fileName}` : `${fileCount} files (${fileName}...)`
  }

  return 'Unknown content'
}

const formatTime = (timestamp: number): string => {
  const now = new Date()
  const date = new Date(timestamp)
  const diffMs = now.getTime() - date.getTime()
  const diffMins = Math.floor(diffMs / (1000 * 60))
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60))
  const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24))

  if (diffMins < 1) {
    return 'just now'
  } else if (diffMins < 60) {
    return `${diffMins} minute${diffMins > 1 ? 's' : ''} ago`
  } else if (diffHours < 24) {
    return `${diffHours} hour${diffHours > 1 ? 's' : ''} ago`
  } else if (diffDays < 7) {
    return `${diffDays} day${diffDays > 1 ? 's' : ''} ago`
  } else {
    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
  }
}

const imageDimensions = ref<{ width: number; height: number } | null>(null)

const handleImageLoad = (event: Event) => {
  const img = event.target as HTMLImageElement
  if (img) {
    imageDimensions.value = {
      width: img.naturalWidth,
      height: img.naturalHeight
    }
  }
}

const parseFilePaths = computed(() => {
  if (
    props.item.primaryType !== 'files' ||
    (!props.item.content.text && !props.item.content.files)
  ) {
    return []
  }

  const textPaths = (props.item.content.text || '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
  const filePaths = (props.item.content.files || []).map(safeDecodeURIComponent)

  return [...textPaths, ...filePaths]
})

const getFileDisplayName = computed(() => {
  const files = parseFilePaths.value
  if (files.length === 0) return ''

  const firstFileName = getFileNameFromPath(files[0])
  return files.length === 1 ? firstFileName : `${firstFileName} 等${files.length}个文件`
})

const getContentSummary = (item: ClipboardItem): string => {
  const content = item.content

  if (item.primaryType === 'files') {
    const fileCount = parseFilePaths.value.length
    if (fileCount > 0) {
      return `${fileCount} 个文件`
    }
    return 'File Path'
  }

  if (content.image) {
    if (imageDimensions.value) {
      return `${imageDimensions.value.width} × ${imageDimensions.value.height}`
    }
    return 'Image'
  }

  if (content.text) {
    return `${content.text.length} character${content.text.length !== 1 ? 's' : ''}`
  }

  if (content.html) {
    const textContent = stripHtmlTags(content.html)
    return `${textContent.length} character${textContent.length !== 1 ? 's' : ''}`
  }

  if (content.rtf) {
    return 'Rich Text'
  }

  return 'Unknown content'
}

const getTypeLabel = (type: string): string => {
  return TYPE_LABEL_MAP[type] || 'Text'
}

const colorValue = computed(() => {
  if (props.item.primaryType === 'color' && props.item.content.text) {
    const text = props.item.content.text.trim()
    if (COLOR_REGEX.test(text)) {
      return text
    }
  }
  return null
})

const getContrastColor = computed(() => {
  if (!colorValue.value) return '#000000'

  const hex = colorValue.value.replace('#', '')
  const r = parseInt(hex.substring(0, 2), 16)
  const g = parseInt(hex.substring(2, 4), 16)
  const b = parseInt(hex.substring(4, 6), 16)
  const yiq = (r * 299 + g * 587 + b * 114) / 1000

  return yiq >= 128 ? '#000000' : '#ffffff'
})

const getCardTheme = (primaryType: string): string => {
  return CARD_THEME_MAP[primaryType] || 'blue'
}

const { showContextMenu } = useContextMenu()

const handleClick = () => {
  emit('click', props.item.id)
}

const handleContextMenu = (event: MouseEvent) => {
  const menuItems: ContextMenuItem[] = [
    {
      label: '粘贴为纯文本',
      icon: IconTextBox,
      action: handleRestoreAsText,
      disabled: !hasConvertibleContent(props.item)
    }
  ]

  if (
    props.item.primaryType === 'image' &&
    props.item.content.ocr &&
    props.item.content.ocr.trim()
  ) {
    menuItems.push({
      label: '复制OCR文字',
      icon: IconTextBox,
      action: handleRestoreOcrText
    })
  }

  if (
    props.item.primaryType === 'image' &&
    (!props.item.content.ocr || !props.item.content.ocr.trim())
  ) {
    menuItems.push({
      label: '解析OCR',
      icon: IconTextBox,
      action: handleParseOcr
    })
  }

  if (props.item.primaryType === 'image') {
    menuItems.push({
      label: '解析二维码',
      icon: IconTextBox,
      action: handleParseQRCode
    })
  }

  if (['text', 'html', 'url'].includes(props.item.primaryType)) {
    menuItems.push({
      label: '生成二维码',
      icon: IconTextBox,
      action: handleGenerateQRCode
    })
  }

  if (props.allTags && props.allTags.length > 0) {
    const currentTagIds = new Set(props.item.tags?.map((tag) => tag.id) || [])
    const tagSubmenu: ContextMenuItem[] = props.allTags.map((tag) => {
      const isTagged = currentTagIds.has(tag.id)
      return {
        label: isTagged ? `${tag.name}` : tag.name,
        icon: isTagged ? IconCheck : IconCircle,
        iconColor: tag.color,
        action: () => (isTagged ? handleRemoveTag(tag.id) : handleAddTag(tag.id))
      }
    })

    if (props.activeTagId !== 'all' && currentTagIds.size > 0) {
      tagSubmenu.push({
        label: '---',
        action: () => {},
        disabled: true
      })

      tagSubmenu.push({
        label: '移除当前标签',
        icon: IconDelete,
        action: handleRemoveCurrentTag
      })
    }

    menuItems.push({
      label: '标签管理',
      icon: IconTag,
      action: () => {},
      submenu: tagSubmenu
    })
  }

  menuItems.push({
    label: '---',
    action: () => {},
    disabled: true
  })

  menuItems.push({
    label: '删除',
    icon: IconDelete,
    action: handleDeleteItem
  })

  showContextMenu(event, menuItems, `clipboard-card-${props.item.id}`)
}

const handleRestoreAsText = () => {
  emit('restoreAsText', props.item.id)
}

const handleRestoreOcrText = () => {
  emit('restoreOcrText', props.item.id)
}

const handleParseQRCode = () => {
  emit('parseQRCode', props.item.id)
}

const handleParseOcr = () => {
  emit('parseOcr', props.item.id)
}

const handleGenerateQRCode = () => {
  emit('generateQRCode', props.item.id)
}

const handleDeleteItem = async () => {
  try {
    const success = await clipboard.deleteItem(props.item.id)
    if (success) {
      emit('delete', props.item.id)
    } else {
      alert('删除失败')
    }
  } catch (error) {
    console.error('删除剪贴板项目失败:', error)
    alert('删除失败: ' + (error as Error).message)
  }
}

const handleAddTag = async (tagId: string) => {
  try {
    await tags.bindTagToRecord(props.item.id, tagId)
  } catch (error) {
    console.error('绑定标签失败:', error)
    alert('绑定标签失败: ' + (error as Error).message)
  }
}

const handleRemoveTag = async (tagId: string) => {
  try {
    await tags.unbindTagFromRecord(props.item.id, tagId)
  } catch (error) {
    console.error('解绑标签失败:', error)
    alert('解绑标签失败: ' + (error as Error).message)
  }
}

const handleRemoveCurrentTag = async () => {
  if (!props.activeTagId || props.activeTagId === 'all') {
    return
  }

  try {
    await tags.unbindTagFromRecord(props.item.id, props.activeTagId)
  } catch (error) {
    console.error('解绑当前标签失败:', error)
    alert('解绑当前标签失败: ' + (error as Error).message)
  }
}

const shortcutLabel = computed(() => {
  if (props.index < 9) {
    const keyNumber = props.index + 1
    const isMac = navigator.userAgent.includes('Mac')
    const modifierKey = isMac ? '⌘' : 'Ctrl'
    return `${modifierKey} + ${keyNumber}`
  }
  return ''
})
</script>

<template>
  <div
    class="clipboard-card"
    :class="[`theme-${getCardTheme(item.primaryType)}`, { selected: isSelected }]"
    @click="handleClick"
    @contextmenu="handleContextMenu"
  >
    <!-- 卡片头部 -->
    <div class="card-header">
      <div class="header-left">
        <span class="type-label">{{ getTypeLabel(item.primaryType) }}</span>
        <span class="time-label">{{ formatTime(item.timestamp) }}</span>
      </div>
      <!-- 应用图标独占右上角 -->
      <div v-if="sourceAppInfo?.icon" class="app-icon">
        <img :src="sourceAppInfo.icon" :alt="sourceAppInfo.application" />
      </div>
    </div>

    <!-- 卡片主要内容 -->
    <div class="card-main">
      <!-- <h3 class="card-title">{{ getContentTitle(item) }}</h3> -->

      <!-- RTF 内容预览 - 如果存在 HTML 则优先使用 HTML -->
      <div class="card-rich-preview" v-if="item.primaryType === 'rtf' && item.content.html">
        <div class="rich-content" v-html="htmlContent"></div>
        <div class="rich-content-overlay"></div>
      </div>

      <!-- RTF 内容预览 - 仅当没有 HTML 时使用 RTF 转换 -->
      <div class="card-rich-preview" v-else-if="item.primaryType === 'rtf' && item.content.rtf">
        <div class="rich-content" v-html="rtfToHtml"></div>
        <div class="rich-content-overlay"></div>
      </div>

      <!-- HTML 内容预览 -->
      <div class="card-rich-preview" v-else-if="item.primaryType === 'html' && item.content.html">
        <div class="rich-content" v-html="htmlContent"></div>
        <div class="rich-content-overlay"></div>
      </div>

      <!-- 颜色预览 -->
      <div
        class="card-color"
        v-else-if="item.primaryType === 'color' && colorValue"
        :style="{ backgroundColor: colorValue || undefined }"
      >
        <div class="color-display">
          <span class="color-text" :style="{ color: getContrastColor }">{{ colorValue }}</span>
        </div>
      </div>

      <!-- 图片预览 -->
      <div class="card-image" v-else-if="item.primaryType === 'image' && item.content.image">
        <ImageDisplay
          :image-data="item.content.image"
          alt="Image preview"
          image-class="card-image-inner"
          @load="handleImageLoad"
        />
      </div>

      <!-- 文件路径预览 -->
      <div class="card-file" v-else-if="item.primaryType === 'files' && parseFilePaths.length > 0">
        <div class="file-icons">
          <!-- 单个文件显示 -->
          <div v-if="parseFilePaths.length === 1" class="file-icon single">
            <IconFile class="file-icon-svg" />
          </div>
          <!-- 多个文件显示 -->
          <div v-else class="file-icon multiple">
            <IconFileMultiple class="file-icon-svg" />
          </div>
        </div>
        <div class="file-name" :title="getFileDisplayName">{{ getFileDisplayName }}</div>
      </div>

      <!-- 其他类型内容预览 -->
      <div
        class="card-preview"
        v-else-if="
          item.primaryType !== 'image' &&
          item.primaryType !== 'color' &&
          item.primaryType !== 'files'
        "
      >
        {{ getContentPreview(item) }}
      </div>
      <div v-else>No Preview</div>
    </div>

    <!-- 卡片底部信息 -->
    <div class="card-footer">
      <!-- 左侧标签颜色点 -->
      <div class="footer-left">
        <div v-if="item.tags && item.tags.length > 0" class="tag-dots">
          <div
            v-for="tag in item.tags"
            :key="tag.id"
            class="tag-dot"
            :style="{ backgroundColor: tag.color }"
            :title="tag.name"
          ></div>
        </div>
      </div>

      <!-- 内容摘要显示（居中显示） -->
      <span class="character-count">{{ getContentSummary(item) }}</span>

      <!-- 快捷选择模式下显示快捷键（右侧） -->
      <div class="footer-right">
        <div v-if="isQuickSelectMode && shortcutLabel" class="shortcut-label">
          {{ shortcutLabel }}
        </div>
      </div>
    </div>

    <!-- 选中指示器 -->
    <div v-if="isSelected" class="selection-indicator"></div>
  </div>
</template>

<style scoped>
.clipboard-card {
  /* CSS 变量定义 */
  --card-main-padding: 12px;

  background: linear-gradient(135deg, #ffffff 0%, #fafbfc 100%);
  border-radius: 16px;
  cursor: pointer;
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  box-shadow:
    0 1px 3px rgba(0, 0, 0, 0.08),
    0 1px 2px rgba(0, 0, 0, 0.06);
  width: 300px;
  height: 76vh;
  flex-shrink: 0;
  border: 1px solid rgb(197 197 197 / 80%);
  backdrop-filter: blur(10px);
}

.clipboard-card:hover {
  transform: translateY(-2px);
  box-shadow:
    0 8px 25px rgba(0, 0, 0, 0.12),
    0 4px 10px rgba(0, 0, 0, 0.08);
  border-color: rgba(255, 255, 255, 0.9);
}

.clipboard-card.selected {
  transform: translateY(-2x);
  box-shadow:
    0 8px 25px rgba(59, 130, 246, 0.25),
    0 4px 10px rgba(59, 130, 246, 0.15);
  border-color: rgba(59, 130, 246, 0.3);
}

/* 卡片主题色 */
.theme-green .type-label {
  background: linear-gradient(135deg, #10b981 0%, #059669 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(16, 185, 129, 0.3);
}

.theme-blue .type-label {
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(59, 130, 246, 0.3);
}

.theme-red .type-label {
  background: linear-gradient(135deg, #ef4444 0%, #dc2626 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(239, 68, 68, 0.3);
}

.theme-orange .type-label {
  background: linear-gradient(135deg, #f59e0b 0%, #d97706 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(245, 158, 11, 0.3);
}

.theme-purple .type-label {
  background: linear-gradient(135deg, #8b5cf6 0%, #7c3aed 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(139, 92, 246, 0.3);
}

.theme-teal .type-label {
  background: linear-gradient(135deg, #06b6d4 0%, #0891b2 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(6, 182, 212, 0.3);
}

.theme-gray .type-label {
  background: linear-gradient(135deg, #6b7280 0%, #4b5563 100%);
  color: white;
  box-shadow: 0 2px 4px rgba(107, 114, 128, 0.3);
}

/* 卡片组件 */
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding-left: 16px;
  padding-right: 16px;
  padding-top: 10px;
  padding-bottom: 4px;
  border-bottom: 1px solid rgba(243, 244, 246, 0.8);
  gap: 12px;
  flex-shrink: 0;
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.9) 0%, rgba(248, 250, 252, 0.9) 100%);
}

.header-left {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
}

.app-icon {
  width: 54px;
  height: 54px;
  overflow: hidden;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgba(255, 255, 255, 0.8);
}

.app-icon img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.type-label {
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding: 6px 12px;
  border-radius: 8px;
  /* background: linear-gradient(135deg, #6b7280 0%, #4b5563 100%); */
  color: white;
  display: inline-block;
  width: fit-content;
  max-width: 80px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  /* transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1); */
  /* border: 1px solid rgba(255, 255, 255, 0.2); */
}

.time-label {
  font-size: 11px;
  color: #64748b;
  font-weight: 600;
  line-height: 1.3;
  margin-top: 4px;
  opacity: 0.8;
  transition: opacity 0.3s ease;
}

.clipboard-card:hover .time-label {
  opacity: 1;
}

.card-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: var(--card-main-padding);
  overflow: hidden;
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.95) 0%, rgba(250, 251, 252, 0.95) 100%);
}

.card-title {
  font-size: 16px;
  font-weight: 700;
  line-height: 1.4;
  margin: 0;
  color: #1e293b;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  letter-spacing: -0.025em;
}

.card-preview {
  font-size: 14px;
  line-height: 1.6;
  color: #475569;
  display: -webkit-box;
  -webkit-line-clamp: 6;
  line-clamp: 6;
  -webkit-box-orient: vertical;
  overflow: hidden;
  font-weight: 500;
  word-wrap: break-word;
}

.card-rich-preview {
  flex: 1;
  overflow: hidden;
  position: relative;
  /* 为遮罩层提供定位参考 */
  /* border-radius: 12px; */
  /* background: linear-gradient(135deg, rgba(248, 250, 252, 0.8) 0%, rgba(241, 245, 249, 0.8) 100%);
  border: 1px solid rgba(226, 232, 240, 0.6);
  backdrop-filter: blur(5px); */
}

.rich-content {
  height: 100%;
  max-height: 200px;
  color: #475569;
  font-size: 14px;
  line-height: 1.6;
  overflow: hidden;
  word-wrap: break-word;
  word-break: break-word;
  font-weight: 500;
}

/* 透明遮罩层，阻止所有交互 */
.rich-content-overlay {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: transparent;
  z-index: 10;
  cursor: inherit;
  /* 继承父元素的cursor样式 */
  /* 阻止所有鼠标和触摸事件 */
  pointer-events: auto;
}

:global(.rich-content > *:first-child) {
  overflow: hidden !important;
  padding: 0 !important;
  margin: 0 !important;
  border: none !important;
}

:global(.rich-content *) {
  overflow: hidden !important;
}

/* 禁用所有可能的交互元素 */
.rich-content a,
.rich-content button,
.rich-content input,
.rich-content select,
.rich-content textarea,
.rich-content area,
.rich-content object,
.rich-content embed,
.rich-content iframe,
.rich-content video,
.rich-content audio,
.rich-content [onclick],
.rich-content [href],
.rich-content [style*='cursor: pointer'] {
  color: #64748b !important;
  text-decoration: none !important;
  cursor: default !important;
  pointer-events: none !important;
  background: none !important;
  border: none !important;
  outline: none !important;
}

.rich-content a:hover,
.rich-content button:hover,
.rich-content input:hover,
.rich-content select:hover,
.rich-content textarea:hover {
  color: #64748b !important;
  text-decoration: none !important;
  background: none !important;
}

.card-image {
  flex: 1;
  display: flex;
  flex-direction: column;
  margin: calc(-1 * var(--card-main-padding));
  overflow: hidden;
  /* background: linear-gradient(135deg, #f8fafc 0%, #f1f5f9 100%);
  border-radius: 12px; */
}

.card-image > :first-child {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}

.card-image img,
.card-image-inner {
  width: 100%;
  height: 100%;
  object-fit: contain;
  /* border-radius: 12px;
  box-shadow:
    0 4px 8px rgba(0, 0, 0, 0.08),
    0 2px 4px rgba(0, 0, 0, 0.06);
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);*/
}

.card-ocr-text {
  padding: 8px 12px;
  background: rgba(248, 250, 252, 0.9);
  border-top: 1px solid rgba(226, 232, 240, 0.8);
  font-size: 11px;
  line-height: 1.4;
  max-height: 80px;
  overflow-y: auto;
}

.ocr-label {
  font-weight: 600;
  color: #64748b;
  margin-bottom: 4px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.ocr-content {
  color: #475569;
  word-break: break-word;
  white-space: pre-wrap;
}

.card-file {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 24px;
  /* background: linear-gradient(135deg, rgba(248, 250, 252, 0.8) 0%, rgba(241, 245, 249, 0.8) 100%); */
  margin: calc(-1 * var(--card-main-padding));
}

.file-icons {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 72px;
  width: 72px;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  border-radius: 16px;
  box-shadow:
    0 4px 8px rgba(59, 130, 246, 0.3),
    0 2px 4px rgba(59, 130, 246, 0.2);
}

.file-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.file-icon:hover {
  transform: scale(1.1);
}

.file-icon-svg {
  width: 36px;
  height: 36px;
  color: white;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.1));
}

.file-name {
  font-size: 15px;
  font-weight: 600;
  color: #1e293b;
  text-align: center;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  letter-spacing: -0.025em;
}

.card-color {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: calc(-1 * var(--card-main-padding));
  padding: var(--card-main-padding);
  /* border-radius: 12px; */
  overflow: hidden;
}

.color-display {
  width: 100%;
  height: 100%;
  min-height: 140px;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  /* border-radius: 12px; */
  /* box-shadow:
    inset 0 2px 4px rgba(0, 0, 0, 0.1),
    0 4px 8px rgba(0, 0, 0, 0.08); */
  transition: transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
}

.color-display:hover {
  transform: scale(1.5);
}

.color-text {
  font-size: 28px;
  font-weight: 700;
  font-family: 'SF Mono', 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  letter-spacing: 0.5px;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
}

.card-footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px;
  border-top: 1px solid rgba(226, 232, 240, 0.8);
  flex-shrink: 0;
  background: linear-gradient(135deg, rgba(248, 250, 252, 0.9) 0%, rgba(241, 245, 249, 0.9) 100%);
  position: relative;
  height: 32px;
  /* 固定高度防止撑高 */
  box-sizing: border-box;
}

.footer-left {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}

.tag-dots {
  display: flex;
  gap: 4px;
  align-items: center;
}

.tag-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 1px solid rgba(255, 255, 255, 0.8);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
  transition: transform 0.2s ease;
  cursor: help;
}

.tag-dot:hover {
  transform: scale(1.2);
}

.character-count {
  font-size: 12px;
  color: #64748b;
  font-weight: 600;
  letter-spacing: 0.025em;
  opacity: 0.8;
  transition: opacity 0.3s ease;
  text-align: center;
  white-space: nowrap;
}

.footer-right {
  flex: 1;
  min-width: 0;
  display: flex;
  justify-content: flex-end;
  align-items: center;
}

.clipboard-card:hover .character-count {
  opacity: 1;
}

.shortcut-label {
  font-size: 10px;
  font-weight: 500;
  letter-spacing: 0.02em;
  padding: 2px 4px;
  border-radius: 3px;
  background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%);
  color: white;
  box-shadow: 0 1px 2px rgba(59, 130, 246, 0.3);
  transition: all 0.2s ease;
  animation: shortcut-pulse 1.5s ease-in-out infinite;
  white-space: nowrap;
  flex-shrink: 0;
  line-height: 1.2;
  max-height: 16px;
  display: flex;
  align-items: center;
}

@keyframes shortcut-pulse {
  0%,
  100% {
    transform: scale(1);
    box-shadow: 0 1px 2px rgba(59, 130, 246, 0.3);
  }

  50% {
    transform: scale(1.02);
    box-shadow: 0 2px 4px rgba(59, 130, 246, 0.4);
  }
}

.selection-indicator {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  border: 3px solid #3b82f6;
  border-radius: 16px;
  pointer-events: none;
  background: linear-gradient(135deg, rgba(59, 130, 246, 0.05) 0%, rgba(37, 99, 235, 0.05) 100%);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.2),
    0 0 0 1px rgba(59, 130, 246, 0.1);
}
</style>
