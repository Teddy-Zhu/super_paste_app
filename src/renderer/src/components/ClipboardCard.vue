<script setup lang="ts">
import type { ClipboardItem, TagGroup, AppInfo } from '@renderer/api/types'
import { computed, ref, watch } from 'vue'
import IconTextBox from '~icons/mdi/text-box'
import IconTag from '~icons/mdi/tag'
import IconCheck from '~icons/mdi/check'
import IconDelete from '~icons/mdi/delete'
import IconCircle from '~icons/mdi/circle'
import IconOpenInNew from '~icons/mdi/open-in-new'
import IconExpand from '~icons/mdi/arrow-expand'
import { useContextMenu, type ContextMenuItem } from '@renderer/composables/useContextMenu'
import { tags, clipboard } from '@renderer/api/renderer-api'
import { send } from '@renderer/api'
import { useDeferredPreview } from '@renderer/composables/useDeferredPreview'
import { createClipboardPreview } from '@renderer/utils/clipboard-preview'
import {
  PREVIEW_ACTIONS,
  PREVIEW_LABELS,
  PREVIEW_TYPES
} from '@renderer/constants/clipboard-preview'
import ClipboardPreviewContent from './ClipboardPreviewContent.vue'
import ClipboardPreviewDialog from './ClipboardPreviewDialog.vue'
import ClipboardSourceApp from './ClipboardSourceApp.vue'

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
const cardRef = ref<HTMLElement>()
const previewVisible = useDeferredPreview(cardRef)

const emit = defineEmits<{
  click: [itemId: string]
  restoreAsText: [itemId: string]
  restoreOcrText: [itemId: string]
  parseOcr: [itemId: string]
  parseQRCode: [itemId: string]
  generateQRCode: [itemId: string]
  delete: [itemId: string]
}>()

const detailsOpen = ref(false)
const imageDimensions = ref<{ width: number; height: number } | null>(null)
// 大文本解析和图片解码继续受可见区域控制。
const preview = computed(() => (previewVisible.value ? createClipboardPreview(props.item) : null))
const presentation = computed(
  () => preview.value || PREVIEW_TYPES[props.item.primaryType] || PREVIEW_TYPES.empty
)
const sourceAppInfo = computed(() =>
  props.item.source_app ? props.apps?.[props.item.source_app] : undefined
)
const summary = computed(() =>
  [
    imageDimensions.value && preview.value?.kind === 'image'
      ? `${imageDimensions.value.width} × ${imageDimensions.value.height}`
      : '',
    preview.value?.summary
  ]
    .filter(Boolean)
    .join(' · ')
)

watch(
  () => props.item.content.image,
  () => {
    imageDimensions.value = null
  }
)
const handleImageLoad = (event: Event): void => {
  const image = event.target as HTMLImageElement
  imageDimensions.value = { width: image.naturalWidth, height: image.naturalHeight }
}

const hasConvertibleContent = (item: ClipboardItem): boolean => {
  const { text, html, rtf, files } = item.content
  return Boolean(text?.trim() || html?.trim() || rtf?.trim() || files?.length)
}

const formatTime = (timestamp: number): string => {
  if (!Number.isFinite(timestamp)) return ''
  const minutes = Math.max(0, Math.floor((Date.now() - timestamp) / 60000))
  if (minutes < 1) return PREVIEW_LABELS.justNow
  if (minutes < 60) return `${minutes} 分钟前`
  if (minutes < 1440) return `${Math.floor(minutes / 60)} 小时前`
  if (minutes < 10080) return `${Math.floor(minutes / 1440)} 天前`
  return new Date(timestamp).toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' })
}

const { showContextMenu } = useContextMenu()

const handleClick = (): void => {
  emit('click', props.item.id)
}

const handleContextMenu = (event: MouseEvent): void => {
  const menuItems: ContextMenuItem[] = [
    {
      label: PREVIEW_ACTIONS.restoreAsText,
      icon: IconTextBox,
      action: handleRestoreAsText,
      disabled: !hasConvertibleContent(props.item)
    }
  ]

  if (preview.value?.kind === 'image' && props.item.content.ocr && props.item.content.ocr.trim()) {
    menuItems.push({
      label: PREVIEW_ACTIONS.restoreOcrText,
      icon: IconTextBox,
      action: handleRestoreOcrText
    })
  }

  if (
    preview.value?.kind === 'image' &&
    (!props.item.content.ocr || !props.item.content.ocr.trim())
  ) {
    menuItems.push({
      label: PREVIEW_ACTIONS.parseOcr,
      icon: IconTextBox,
      action: handleParseOcr
    })
  }

  if (preview.value?.kind === 'image') {
    menuItems.push({
      label: PREVIEW_ACTIONS.parseQRCode,
      icon: IconTextBox,
      action: handleParseQRCode
    })
  }

  if (['text', 'html', 'url'].includes(props.item.primaryType)) {
    menuItems.push({
      label: PREVIEW_ACTIONS.generateQRCode,
      icon: IconTextBox,
      action: handleGenerateQRCode
    })
  }

  if (preview.value?.url && preview.value.kind === 'url') {
    menuItems.push({
      label: PREVIEW_ACTIONS.openInBrowser,
      icon: IconOpenInNew,
      action: handleOpenInBrowser
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
        action: () => undefined,
        disabled: true
      })

      tagSubmenu.push({
        label: PREVIEW_ACTIONS.removeCurrentTag,
        icon: IconDelete,
        action: handleRemoveCurrentTag
      })
    }

    menuItems.push({
      label: PREVIEW_ACTIONS.manageTags,
      icon: IconTag,
      action: () => undefined,
      submenu: tagSubmenu
    })
  }

  menuItems.push({
    label: '---',
    action: () => undefined,
    disabled: true
  })

  menuItems.push({
    label: PREVIEW_ACTIONS.delete,
    icon: IconDelete,
    action: handleDeleteItem
  })

  showContextMenu(event, menuItems, `clipboard-card-${props.item.id}`)
}

const handleRestoreAsText = (): void => {
  emit('restoreAsText', props.item.id)
}

const handleRestoreOcrText = (): void => {
  emit('restoreOcrText', props.item.id)
}

const handleParseQRCode = (): void => {
  emit('parseQRCode', props.item.id)
}

const handleParseOcr = (): void => {
  emit('parseOcr', props.item.id)
}

const handleGenerateQRCode = (): void => {
  emit('generateQRCode', props.item.id)
}

const handleOpenInBrowser = (): void => {
  const url = preview.value?.url?.href
  if (!url) return
  send('open-external-url', url)
}

const handleDeleteItem = async (): Promise<void> => {
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

const handleAddTag = async (tagId: string): Promise<void> => {
  try {
    await tags.bindTagToRecord(props.item.id, tagId)
  } catch (error) {
    console.error('绑定标签失败:', error)
    alert('绑定标签失败: ' + (error as Error).message)
  }
}

const handleRemoveTag = async (tagId: string): Promise<void> => {
  try {
    await tags.unbindTagFromRecord(props.item.id, tagId)
  } catch (error) {
    console.error('解绑标签失败:', error)
    alert('解绑标签失败: ' + (error as Error).message)
  }
}

const handleRemoveCurrentTag = async (): Promise<void> => {
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
    ref="cardRef"
    class="clipboard-card"
    :class="[`theme-${presentation.theme}`, { selected: isSelected }]"
    @click="handleClick"
    @contextmenu="handleContextMenu"
  >
    <div class="card-header">
      <div class="header-left">
        <span class="type-label">{{ presentation.label }}</span>
        <time class="time-label" :title="new Date(item.timestamp).toLocaleString('zh-CN')">{{
          formatTime(item.timestamp)
        }}</time>
      </div>
      <ClipboardSourceApp v-if="sourceAppInfo" :app="sourceAppInfo" />
    </div>

    <div class="preview-toolbar">
      <span
        class="content-formats"
        :title="`${PREVIEW_LABELS.formats} · ${preview?.formats.join(' · ') || presentation.label}`"
        >{{ preview?.formats.join(' · ') }}</span
      >
      <button
        v-if="preview && preview.kind !== 'empty'"
        type="button"
        class="details-button"
        @click.stop="detailsOpen = true"
        @keydown.enter.stop
        @keydown.space.stop
      >
        {{ PREVIEW_LABELS.details }}<IconExpand />
      </button>
    </div>

    <div class="card-main">
      <ClipboardPreviewContent v-if="preview" :preview="preview" @image-load="handleImageLoad" />
      <div v-else class="preview-placeholder" aria-hidden="true">
        <span></span><span></span><span></span>
      </div>
    </div>

    <div class="card-footer">
      <div
        v-if="item.tags?.length"
        class="tag-dots"
        :title="item.tags.map((tag) => tag.name).join('、')"
      >
        <span
          v-for="tag in item.tags.slice(0, 3)"
          :key="tag.id"
          class="tag-dot"
          :style="{ backgroundColor: tag.color }"
        ></span>
        <span v-if="item.tags.length > 3" class="tag-overflow">+{{ item.tags.length - 3 }}</span>
      </div>
      <span class="content-summary" :title="summary">{{ summary }}</span>
      <span
        v-if="shortcutLabel"
        class="shortcut-label"
        :class="{ visible: isQuickSelectMode }"
        :aria-hidden="!isQuickSelectMode"
        >{{ shortcutLabel }}</span
      >
    </div>
    <div v-if="isSelected" class="selection-indicator"></div>
    <ClipboardPreviewDialog
      v-if="detailsOpen && preview"
      :preview="preview"
      :source="sourceAppInfo"
      :summary="summary"
      @close="detailsOpen = false"
    />
  </div>
</template>

<style scoped>
.clipboard-card {
  --type-color: #2563eb;
  background: var(--color-bg-primary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  box-shadow: var(--shadow-soft-xs);
  width: min(300px, calc(100vw - 24px));
  height: var(--clipboard-card-height, calc(100vh - 66px));
  flex-shrink: 0;
  border: 1px solid var(--color-control-border);
  contain: layout style paint;
}
.clipboard-card:hover {
  border-color: var(--color-primary);
  box-shadow: var(--shadow-soft-sm);
}
.clipboard-card.selected {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 1px var(--color-primary);
}
.clipboard-card.selected:hover {
  border-color: var(--color-primary-hover);
  box-shadow: 0 0 0 1px var(--color-primary-hover);
}
.clipboard-card:hover .card-header {
  background: var(--color-selection);
}
.clipboard-card:active,
.clipboard-card.selected:active {
  border-color: var(--color-primary-active);
  box-shadow: inset 0 0 0 1px var(--color-primary-active);
}
.clipboard-card:active .card-header {
  background: var(--color-selection-hover);
}
.theme-green {
  --type-color: #059669;
}
.theme-red {
  --type-color: #dc2626;
}
.theme-orange {
  --type-color: #d97706;
}
.theme-purple {
  --type-color: #7c3aed;
}
.theme-teal {
  --type-color: #0891b2;
}
.theme-gray {
  --type-color: #64748b;
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 10px 12px 8px;
  flex-shrink: 0;
}
.header-left {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 5px;
  min-width: 0;
}
.type-label {
  font-size: 11px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--type-color);
  white-space: nowrap;
}
.type-label::before {
  content: '';
  width: 5px;
  height: 5px;
  border-radius: 2px;
  background: currentColor;
}
.time-label {
  font-size: 11px;
  color: #64748b;
}
.preview-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 25px;
  padding: 0 10px 5px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--color-border-light);
}
.content-formats {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  color: var(--color-text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.details-button {
  display: flex;
  gap: 4px;
  align-items: center;
  flex-shrink: 0;
  padding: 2px 5px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: #64748b;
  font-size: 11px;
  cursor: pointer;
}
.details-button:hover {
  color: var(--color-primary);
  background: var(--color-selection);
}
.details-button:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 1px;
}
.card-main {
  flex: 1;
  min-height: 0;
  min-width: 0;
  padding: 12px;
  overflow: hidden;
}
.card-main :deep(.kind-text),
.card-main :deep(.kind-code),
.card-main :deep(.kind-rich) {
  mask-image: linear-gradient(#000 calc(100% - 14px), transparent);
}
.preview-placeholder {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-top: 6px;
}
.preview-placeholder span {
  height: 10px;
  border-radius: 3px;
  background: #f1f5f9;
}
.preview-placeholder span:last-child {
  width: 60%;
}
.card-footer {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 10px;
  height: 40px;
  box-sizing: border-box;
  border-top: 1px solid #e2e8f0;
  flex-shrink: 0;
  background: var(--color-bg-tertiary);
}
.tag-dots {
  display: flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
}
.tag-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  border: 1px solid #ffffffcc;
}
.tag-overflow {
  font-size: 9px;
  color: #64748b;
}
.content-summary {
  min-width: 0;
  flex: 1;
  font-size: 11px;
  color: #64748b;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.shortcut-label {
  visibility: hidden;
  opacity: 0;
  translate: 0 2px;
  padding: 2px 4px;
  border-radius: 3px;
  border: 1px solid var(--color-control-border);
  background: var(--color-bg-primary);
  color: var(--color-text-secondary);
  box-shadow: 0 1px 0 var(--color-control-border);
  font-size: 11px;
  line-height: 16px;
  white-space: nowrap;
  flex-shrink: 0;
  pointer-events: none;
  transition:
    opacity var(--duration-fast),
    translate var(--duration-fast) var(--ease-out),
    visibility var(--duration-fast);
}
.shortcut-label.visible {
  visibility: visible;
  opacity: 1;
  translate: 0 0;
}
.selection-indicator {
  position: absolute;
  inset: 0;
  border: 1px solid var(--color-primary);
  border-radius: inherit;
  pointer-events: none;
}
</style>
