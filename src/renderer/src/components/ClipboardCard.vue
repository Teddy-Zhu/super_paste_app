<script setup lang="ts">
import type { ClipboardItem, TagGroup, AppInfo } from '@renderer/api/types'
import { computed, onUnmounted, ref, shallowRef, watch } from 'vue'
import IconExpand from '~icons/mdi/arrow-expand'
import { useContextMenu } from '@renderer/composables/useContextMenu'
import { createClipboardPreview } from '@renderer/features/clipboard/preview'
import { createClipboardMenu } from '@renderer/features/clipboard/menu'
import { PREVIEW_LABELS, PREVIEW_TYPES } from '@renderer/constants/clipboard-preview'
import { formatByteSize } from '@renderer/features/clipboard/preview/providers/image'
import { clipboard } from '@renderer/api/renderer-api'
import { showMessage } from '@renderer/api/dialog'
import ClipboardPreviewContent from './ClipboardPreviewContent.vue'
import ClipboardPreviewDialog from './ClipboardPreviewDialog.vue'
import ClipboardSourceApp from './ClipboardSourceApp.vue'
import type { OCRTaskStatus } from '@shared/ai-ocr'
import { OCR_MANAGE_TEXT } from '@renderer/constants/ocr'

interface Props {
  item: ClipboardItem
  previewsActive: boolean
  overlayActive: boolean
  index: number
  isSelected: boolean
  isQuickSelectMode: boolean
  allTags?: TagGroup[]
  activeTagId?: string
  apps?: Record<string, AppInfo>
  ocrStatus?: OCRTaskStatus
}

const props = defineProps<Props>()
const ocrBusy = computed(
  () => props.ocrStatus?.state === 'queued' || props.ocrStatus?.state === 'running'
)
const ocrStatusText = computed(() => {
  if (props.ocrStatus?.state === 'queued') return OCR_MANAGE_TEXT.queued
  if (props.ocrStatus?.state === 'running') return OCR_MANAGE_TEXT.running
  if (props.ocrStatus?.state === 'error') return props.ocrStatus.error || OCR_MANAGE_TEXT.error
  return ''
})
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
const fullItem = shallowRef<ClipboardItem>()
const imageDimensions = ref<{ width: number; height: number } | null>(null)
const detailsLoading = ref(false)
let detailRequestId = 0
let componentActive = true
// 列表已按视口虚拟化，挂载卡片即准备预览，让图片读取与面板动画并行。
const preview = computed(() => (props.previewsActive ? createClipboardPreview(props.item) : null))
const detailPreview = computed(() =>
  detailsOpen.value && fullItem.value ? createClipboardPreview(fullItem.value) : null
)
const presentation = computed(
  () => preview.value || PREVIEW_TYPES[props.item.primaryType] || PREVIEW_TYPES.empty
)
const sourceAppInfo = computed(() =>
  props.item.source_app ? props.apps?.[props.item.source_app] : undefined
)
const summary = computed(() => {
  const item = props.item
  if (preview.value?.kind === 'image' || (!preview.value && item.primaryType === 'image')) {
    return [
      PREVIEW_LABELS.image,
      typeof item.imageBytes === 'number' ? formatByteSize(item.imageBytes) : ''
    ]
      .filter(Boolean)
      .join(' · ')
  }
  if (preview.value?.kind === 'files' && typeof item.filesCount === 'number') {
    return `${item.filesCount.toLocaleString()} 个文件`
  }
  return preview.value?.summary || ''
})
const detailSummary = computed(() => {
  const item = fullItem.value
  const preview = detailPreview.value
  if (!item || !preview) return ''
  if (preview.kind === 'image') {
    return [
      PREVIEW_LABELS.image,
      imageDimensions.value
        ? `${imageDimensions.value.width} × ${imageDimensions.value.height}`
        : '',
      typeof item.imageBytes === 'number' ? formatByteSize(item.imageBytes) : ''
    ]
      .filter(Boolean)
      .join(' · ')
  }
  if (preview.kind === 'files' && typeof item.filesCount === 'number') {
    return `${item.filesCount.toLocaleString()} 个文件`
  }
  return preview.summary
})

const formatLabels: Record<string, string> = {
  text: PREVIEW_LABELS.text,
  html: 'HTML',
  image: PREVIEW_LABELS.image,
  rtf: 'RTF',
  files: '文件'
}
const contentFormats = computed(() =>
  props.item.availableFormats.map((format) => formatLabels[format] || format).join(' · ')
)

watch([() => props.item.id, () => props.item.contentVersion], () => {
  detailRequestId++
  detailsOpen.value = false
  detailsLoading.value = false
  fullItem.value = undefined
  imageDimensions.value = null
})

onUnmounted(() => {
  componentActive = false
  detailRequestId++
  fullItem.value = undefined
  imageDimensions.value = null
})

watch(
  [() => props.previewsActive, () => props.overlayActive],
  ([previewsActive, overlayActive]) => {
    if (!previewsActive || !overlayActive) closeDetails()
  }
)

const closeDetails = (): void => {
  detailsOpen.value = false
  fullItem.value = undefined
  imageDimensions.value = null
  detailsLoading.value = false
  detailRequestId++
}

const openDetails = async (): Promise<void> => {
  const requestId = ++detailRequestId
  const { id, contentVersion } = props.item
  detailsLoading.value = true
  try {
    const result = await clipboard.getItem(id)
    if (
      requestId !== detailRequestId ||
      !componentActive ||
      props.item.id !== id ||
      props.item.contentVersion !== contentVersion
    )
      return
    fullItem.value = result
    detailsOpen.value = true
  } catch (error) {
    if (requestId === detailRequestId && componentActive) {
      await showMessage(
        `读取剪贴板详情失败: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  } finally {
    if (requestId === detailRequestId) detailsLoading.value = false
  }
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

const handleImageLoad = (event: Event): void => {
  if (!detailsOpen.value || !fullItem.value) return
  const image = event.target as HTMLImageElement
  imageDimensions.value = { width: image.naturalWidth, height: image.naturalHeight }
}

const handleContextMenu = (event: MouseEvent): void => {
  event.preventDefault()
  const item = props.item
  const menuItems = createClipboardMenu({
    item,
    preview: createClipboardPreview(item),
    ocrBusy: ocrBusy.value,
    allTags: props.allTags,
    activeTagId: props.activeTagId,
    callbacks: {
      restoreAsText: (itemId) => emit('restoreAsText', itemId),
      restoreOcrText: (itemId) => emit('restoreOcrText', itemId),
      parseOcr: (itemId) => emit('parseOcr', itemId),
      parseQRCode: (itemId) => emit('parseQRCode', itemId),
      generateQRCode: (itemId) => emit('generateQRCode', itemId),
      deleted: (itemId) => emit('delete', itemId)
    }
  })
  showContextMenu(event, menuItems, `clipboard-card-${item.id}`)
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
    :class="[`theme-${presentation.theme}`, { selected: isSelected }]"
    :aria-busy="detailsLoading"
    @click="handleClick"
    @contextmenu="handleContextMenu"
  >
    <div class="card-header">
      <div class="header-left">
        <span class="type-label">{{ presentation.label }}</span>
        <span
          v-if="ocrStatusText"
          class="time-label ocr-status"
          :class="{ 'ocr-error': ocrStatus?.state === 'error' }"
          :title="ocrStatus?.error"
          role="status"
          >{{ ocrStatusText }}</span
        >
        <time class="time-label" :title="new Date(item.timestamp).toLocaleString('zh-CN')">{{
          formatTime(item.timestamp)
        }}</time>
      </div>
      <ClipboardSourceApp v-if="sourceAppInfo" :app="sourceAppInfo" />
    </div>

    <div class="preview-toolbar">
      <span
        class="content-formats"
        :title="`${PREVIEW_LABELS.formats} · ${contentFormats || presentation.label}`"
        >{{ contentFormats }}</span
      >
      <button
        v-if="item.availableFormats.length"
        type="button"
        class="details-button"
        :disabled="detailsLoading"
        @click.stop="openDetails"
        @keydown.enter.stop
        @keydown.space.stop
      >
        {{ detailsLoading ? '加载中' : PREVIEW_LABELS.details }}<IconExpand />
      </button>
    </div>

    <div class="card-main">
      <ClipboardPreviewContent v-if="preview" :preview="preview" />
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
    <ClipboardPreviewDialog
      v-if="detailsOpen && detailPreview && overlayActive"
      :preview="detailPreview"
      :source="sourceAppInfo"
      :summary="detailSummary"
      @image-load="handleImageLoad"
      @close="closeDetails"
    />
  </div>
</template>

<style scoped>
.clipboard-card {
  --type-color: var(--color-type-blue);
  --card-border-width: 1px;
  --card-border-color: var(--color-control-border);
  background: var(--color-bg-primary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: box-shadow var(--duration-fast);
  /* 固定信息区域，图片和任务状态不参与各行高度的计算。 */
  display: grid;
  grid-template-rows: 68px 26px minmax(0, 1fr) 40px;
  position: relative;
  overflow: hidden;
  box-shadow: var(--shadow-soft-xs);
  width: min(300px, calc(100vw - 24px));
  height: var(--clipboard-card-height, calc(100vh - 66px));
  flex-shrink: 0;
  border: 0;
  padding: 1px;
  contain: layout style;
}
/* Paint one frame above the content without changing its layout in each state. */
.clipboard-card::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 1;
  border: var(--card-border-width) solid var(--card-border-color);
  border-radius: inherit;
  pointer-events: none;
  transition: border-color var(--duration-fast);
}
.clipboard-card:hover {
  --card-border-width: 2px;
  --card-border-color: var(--color-primary);
  box-shadow: var(--shadow-soft-sm);
}
.clipboard-card.selected {
  --card-border-width: 3px;
  --card-border-color: var(--color-primary);
  box-shadow: var(--shadow-soft-sm);
}
.clipboard-card.selected:hover {
  --card-border-color: var(--color-primary-hover);
  box-shadow: var(--shadow-soft-sm);
}
.clipboard-card:hover .card-header {
  background: var(--color-selection);
}
.clipboard-card.selected .card-header {
  background: var(--color-selection-hover);
}
.clipboard-card:active,
.clipboard-card.selected:active {
  --card-border-color: var(--color-primary-active);
  box-shadow: var(--shadow-soft-xs);
}
.clipboard-card:active .card-header {
  background: var(--color-selection-hover);
}
.theme-green {
  --type-color: var(--color-type-green);
}
.theme-red {
  --type-color: var(--color-type-red);
}
.theme-orange {
  --type-color: var(--color-type-orange);
}
.theme-purple {
  --type-color: var(--color-type-purple);
}
.theme-teal {
  --type-color: var(--color-type-teal);
}
.theme-gray {
  --type-color: var(--color-type-gray);
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 10px 12px 8px;
  min-height: 0;
}
.header-left {
  flex: 1;
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  align-items: center;
  column-gap: 6px;
  row-gap: 5px;
  min-width: 0;
}
.header-left > time {
  grid-column: 1 / -1;
  grid-row: 2;
}
.ocr-status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.time-label.ocr-error {
  color: var(--color-danger);
}
.type-label {
  font-size: 11px;
  line-height: 15px;
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
  line-height: 15px;
  white-space: nowrap;
  color: var(--color-text-secondary);
}
.preview-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 0;
  padding: 0 10px 5px;
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
  color: var(--color-text-secondary);
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
  background: var(--color-bg-secondary);
}
.preview-placeholder span:last-child {
  width: 60%;
}
.card-footer {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 10px;
  min-height: 0;
  box-sizing: border-box;
  border-top: 1px solid var(--color-border-light);
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
  border: 1px solid var(--color-bg-primary);
}
.tag-overflow {
  font-size: 9px;
  color: var(--color-text-secondary);
}
.content-summary {
  min-width: 0;
  flex: 1;
  font-size: 11px;
  color: var(--color-text-secondary);
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
</style>
