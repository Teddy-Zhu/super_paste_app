<script setup lang="ts">
import { ref, shallowRef, computed, nextTick, watch, onMounted, onUnmounted } from 'vue'
import { send, onSimple } from '@renderer/api'
import { clipboard } from '@renderer/api/renderer-api'
import { useSettingsStore } from '@renderer/stores/common'
import type { ClipboardItem, ClipboardHistoryQueryOptions, AppInfo } from '@renderer/api/types'
import { OverlayError, OverlayErrorCode, isClipboardItem } from '@renderer/types/overlay'
import { reconcileClipboardHistory } from '@renderer/utils/clipboard-history'
import type { OCRTaskStatus } from '@shared/ai-ocr'

// 导入优化后的 composables
import {
  useAnimation,
  AnimationState,
  type AnimationControls
} from '@renderer/composables/useAnimation'
import type { OverlayRendererTrace } from '@shared/overlay-events'
import { useSearchAndNavigation } from '@renderer/composables/useSearchAndNavigation'
import { useScrollAndTags, type ScrollAndTagsManager } from '@renderer/composables/useScrollAndTags'
import { useEventManager, useThrottle } from '@renderer/composables/useUtils'
import { useContextMenu } from '@renderer/composables/useContextMenu'
import { useTabIndicator } from '@renderer/composables/useTabIndicator'

// 导入组件
import ClipboardCard from '@renderer/components/ClipboardCard.vue'
import ContextMenu from '@renderer/components/ContextMenu.vue'

// 导入图标
import IconSettings from '~icons/mdi/cog'
import IconDelete from '~icons/mdi/delete'
import IconClipboard from '~icons/mdi/clipboard-text-outline'
import { OVERLAY_LABELS } from '@renderer/constants/desktop-ui'

// ==================== DOM 引用 ====================
const overlayContentRef = ref<HTMLElement>()
const clipboardScrollRef = ref<HTMLElement>()
const searchContainerRef = ref<HTMLElement>()
const searchInputRef = ref<HTMLInputElement>()
const tabsContainerRef = ref<HTMLElement>()

// ==================== 核心状态 ====================
const clipboardHistory = shallowRef<ClipboardItem[]>([])
const clipboardApps = shallowRef<Record<string, AppInfo>>({})
const ocrStatuses = ref<Record<string, OCRTaskStatus>>({})
const selectedIndex = ref(0)
const previewsActive = ref(true)
let previewIdleTimer: ReturnType<typeof setTimeout> | undefined
let ocrExpiryTimer: ReturnType<typeof setInterval> | undefined
const ocrErrorsExpireAt = new Map<string, number>()

const removeOCRStatus = (id: string): void => {
  delete ocrStatuses.value[id]
  ocrErrorsExpireAt.delete(id)
}
const pruneOCRStatuses = (): void => {
  const ids = new Set(clipboardHistory.value.map((item) => item.id))
  for (const id of Object.keys(ocrStatuses.value)) {
    if (!ids.has(id) || (ocrErrorsExpireAt.get(id) ?? Infinity) <= Date.now()) removeOCRStatus(id)
  }
}
const retainPreviews = (): void => {
  clearTimeout(previewIdleTimer)
  previewIdleTimer = undefined
  previewsActive.value = true
}
const schedulePreviewRelease = (): void => {
  clearTimeout(previewIdleTimer)
  previewIdleTimer = setTimeout(() => {
    previewsActive.value = false
  }, 2 * 60000)
}

// ==================== 快捷选择状态 ====================
const isQuickSelectMode = ref(false)

// ==================== 下拉菜单状态 ====================
const isDropdownVisible = ref(false)

// ==================== 事件管理器 ====================
const eventManager = useEventManager()

// ==================== 监听器状态管理 ====================
let transitionId = 0
let animationVersion = 0
let searchAnimationVersion = 0
let historyRequestId = 0
let appliedHistoryFilterKey: string | null = null
let pendingHistoryLoad: { filterKey: string; promise: Promise<boolean> } | undefined
let overlayVisible = false
let preparedOpening: { id: number; animation: AnimationControls } | null = null
let closingTransitionId: number | null = null
let historyDirty = false
let wheelHandler: ((event: WheelEvent) => void) | null = null
const ipcUnsubscribers: Array<() => void> = []

// ==================== 初始化 Composables ====================
const animationManager = useAnimation(overlayContentRef, searchContainerRef, tabsContainerRef)
const { hideContextMenu } = useContextMenu()

const traceOverlay = (
  event: OverlayRendererTrace['event'],
  details: Partial<OverlayRendererTrace> = {}
): void => {
  send('overlay-trace', {
    event,
    transitionId,
    animationVersion,
    animationState: animationManager.currentAnimationState.value,
    sentAt: Date.now(),
    overlayVisible,
    documentHidden: document.hidden,
    documentFocused: document.hasFocus(),
    ...details
  } satisfies OverlayRendererTrace)
}

type RestoreMethod =
  | 'restore'
  | 'restoreAsText'
  | 'restoreOcrText'
  | 'parseQRCode'
  | 'generateQRCode'
type OverlayPosition = 'top' | 'bottom' | 'left' | 'right'

const settingsStore = useSettingsStore()
const overlayPosition = ref<OverlayPosition>(settingsStore.overlayPosition)

const normalizeOverlayPosition = (value: unknown): OverlayPosition => {
  if (value === 'top' || value === 'bottom' || value === 'left' || value === 'right') {
    return value
  }
  return 'bottom'
}

const applyOverlayPosition = (value: unknown): void => {
  const normalized = normalizeOverlayPosition(value)
  overlayPosition.value = normalized
  animationManager.setSlideDirection(normalized)
}

const isSideLayout = computed(() => {
  return overlayPosition.value === 'left' || overlayPosition.value === 'right'
})

// ==================== 剪贴板历史加载 ====================

const getHistoryFilterKey = (text: string, tagIds: string[]): string => {
  return JSON.stringify([text.trim(), [...tagIds].sort()])
}

const getCurrentHistoryFilterKey = (): string => {
  return getHistoryFilterKey(getCurrentSearchText(), getActiveTagIds())
}

/**
 * 获取剪贴板历史记录（带类型验证和错误处理）
 */
const queryClipboardHistory = async (options: ClipboardHistoryQueryOptions): Promise<boolean> => {
  const requestId = ++historyRequestId
  try {
    const defaultOptions: ClipboardHistoryQueryOptions = {
      limit: 200,
      searchText: getCurrentSearchText(),
      tagIds: getActiveTagIds(),
      orderBy: 'last_used',
      orderDirection: 'DESC'
    }

    const queryOptions = { ...defaultOptions, ...options }
    queryOptions.searchText = queryOptions.searchText?.trim() ?? ''
    const filterKey = getHistoryFilterKey(queryOptions.searchText ?? '', queryOptions.tagIds ?? [])
    const result = await clipboard.searchHistory(queryOptions)
    // 输入一变化就拒绝旧筛选的响应，不等待下一次防抖查询启动。
    if (
      requestId !== historyRequestId ||
      filterKey !== getCurrentHistoryFilterKey() ||
      searchAndNavManager.isComposing.value
    )
      return false
    if (!Array.isArray(result.items)) {
      throw new OverlayError(
        '剪贴板历史数据格式无效：不是数组',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { receivedType: typeof result.items, queryOptions }
      )
    }

    const selectedId = clipboardHistory.value[selectedIndex.value]?.id
    const filterChanged = appliedHistoryFilterKey !== filterKey
    const reconciled = reconcileClipboardHistory(
      { items: clipboardHistory.value, apps: clipboardApps.value },
      { items: result.items.filter(isClipboardItem), apps: result.apps }
    )
    clipboardApps.value = reconciled.apps
    const validIds = new Set(reconciled.items.map((item) => item.id))
    for (const id of Object.keys(ocrStatuses.value)) {
      if (!validIds.has(id)) removeOCRStatus(id)
    }
    const preservedIndex = reconciled.items.findIndex((item) => item.id === selectedId)
    selectedIndex.value = filterChanged
      ? 0
      : preservedIndex >= 0
        ? preservedIndex
        : Math.max(0, Math.min(selectedIndex.value, reconciled.items.length - 1))
    appliedHistoryFilterKey = filterKey
    if (reconciled.items === clipboardHistory.value && !filterChanged) return true
    clipboardHistory.value = reconciled.items

    nextTick(() => {
      scrollAndTagsManager.initializeScrollbar()
      scrollAndTagsManager.scrollToSelectedItem()
    })
    return true
  } catch (error) {
    if (requestId !== historyRequestId) return false
    console.error('获取剪贴板历史失败:', error)
    return false
  }
}

const loadClipboardHistory = (options?: ClipboardHistoryQueryOptions): Promise<boolean> => {
  const filterKey = getHistoryFilterKey(
    options?.searchText ?? getCurrentSearchText(),
    options?.tagIds ?? getActiveTagIds()
  )
  const promise = queryClipboardHistory(options ?? {})
  pendingHistoryLoad = { filterKey, promise }
  promise.finally(() => {
    if (pendingHistoryLoad?.promise === promise) pendingHistoryLoad = undefined
  })
  return promise
}

const getActiveTagIds = (): string[] => {
  return scrollAndTagsManager.activeTagId.value === 'all'
    ? []
    : [scrollAndTagsManager.activeTagId.value]
}

const getCurrentSearchText = (): string => {
  return searchAndNavManager.searchText.value || ''
}

const throttledLoadClipboardHistory = useThrottle(() => {
  if (!overlayVisible || animationManager.isAnimating()) {
    historyDirty = true
    return
  }
  historyDirty = false
  const currentTagIds = getActiveTagIds()
  const currentSearchText = getCurrentSearchText()

  loadClipboardHistory({
    tagIds: currentTagIds,
    searchText: currentSearchText
  }).catch(console.error)
}, 500)

// ==================== 临时占位符，将在 searchAndNavManager 定义后重新定义 ====================
let scrollAndTagsManager!: ScrollAndTagsManager

// ==================== 核心功能函数 ====================

/**
 * 通用剪贴板还原函数
 */
const ensureValidItemId = (itemId: string): void => {
  if (!itemId || typeof itemId !== 'string') {
    throw new OverlayError('无效的剪贴板项目ID', OverlayErrorCode.INVALID_STATE, {
      itemId,
      type: typeof itemId
    })
  }
}

const normalizeAfterAction = async (): Promise<void> => {
  await closeWindow()
  if (scrollAndTagsManager.activeTagId.value !== 'all') {
    await scrollAndTagsManager.switchTag('all')
  }
}

const getRestoreMethodMeta = (
  restoreMethod: RestoreMethod
): { typeName: string; action: string } => {
  const typeNameMap: Record<RestoreMethod, string> = {
    restore: '',
    restoreAsText: '纯文本',
    restoreOcrText: 'OCR文字',
    parseQRCode: '二维码',
    generateQRCode: '二维码'
  }

  const actionMap: Record<RestoreMethod, string> = {
    restore: '还原',
    restoreAsText: '还原',
    restoreOcrText: '还原',
    parseQRCode: '解析',
    generateQRCode: '生成'
  }

  return {
    typeName: typeNameMap[restoreMethod],
    action: actionMap[restoreMethod]
  }
}

const restoreClipboard = async (itemId: string, restoreMethod: RestoreMethod): Promise<void> => {
  const actionTransitionId = transitionId
  try {
    ensureValidItemId(itemId)

    if (restoreMethod === 'generateQRCode') {
      const result = await clipboard.generateQRCode(itemId)
      if (result.success) {
        if (actionTransitionId === transitionId && overlayVisible) await normalizeAfterAction()
      } else {
        throw new OverlayError(
          `生成二维码失败: ${result.message}`,
          OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
          { itemId }
        )
      }
    } else {
      const success = await clipboard[restoreMethod](itemId)

      if (success) {
        if (actionTransitionId === transitionId && overlayVisible) await normalizeAfterAction()
      } else {
        const { typeName, action } = getRestoreMethodMeta(restoreMethod)
        throw new OverlayError(
          `剪贴板${typeName}${action}操作失败`,
          OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
          { itemId }
        )
      }
    }
  } catch (error) {
    const { typeName, action } = getRestoreMethodMeta(restoreMethod)
    const suffix = restoreMethod === 'restore' ? '' : ` ${typeName}`
    console.error(`[前端] ${action}剪贴板${suffix}失败:`, error)
  }
}

/**
 * 还原剪贴板内容
 */
const restoreClipboardContent = (itemId: string): Promise<void> =>
  restoreClipboard(itemId, 'restore')

/**
 * 还原剪贴板内容为纯文本
 */
const restoreClipboardContentAsText = (itemId: string): Promise<void> =>
  restoreClipboard(itemId, 'restoreAsText')

/**
 * 还原剪贴板内容为OCR文字
 */
const restoreClipboardContentAsOcrText = (itemId: string): Promise<void> =>
  restoreClipboard(itemId, 'restoreOcrText')

/**
 * 解析剪贴板项目的OCR文字
 */
const parseOcrFromClipboardItem = async (itemId: string): Promise<void> => {
  try {
    ensureValidItemId(itemId)

    const success = await clipboard.parseOcr(itemId)
    if (!success) {
      console.error('[前端] 解析OCR失败')
    }
  } catch (error) {
    console.error('[前端] 解析OCR失败:', error)
  }
}

/**
 * 解析剪贴板项目中的二维码
 */
const parseQRCodeFromClipboardItem = (itemId: string): Promise<void> =>
  restoreClipboard(itemId, 'parseQRCode')

/**
 * 生成剪贴板项目的二维码
 */
const generateQRCodeFromClipboardItem = (itemId: string): Promise<void> =>
  restoreClipboard(itemId, 'generateQRCode')

/**
 * 删除剪贴板项目
 */
const deleteClipboardItem = async (itemId: string): Promise<void> => {
  try {
    ensureValidItemId(itemId)

    throttledLoadClipboardHistory()
  } catch (error) {
    console.error('[前端] 删除剪贴板项目失败:', error)
  }
}

/**
 * 切换下拉菜单显示状态
 */
const toggleDropdownMenu = (): void => {
  isDropdownVisible.value = !isDropdownVisible.value
}

/**
 * 隐藏下拉菜单
 */
const hideDropdownMenu = (): void => {
  isDropdownVisible.value = false
}

/**
 * 打开设置窗口
 */
const openSettings = (): void => {
  try {
    hideDropdownMenu()
    // 关闭当前覆盖层窗口
    closeWindow()
    // 发送消息到主进程打开设置窗口
    send('open-main-window')
  } catch (error) {
    console.error('[前端] 打开设置失败:', error)
  }
}

/**
 * 清空剪贴板历史
 */
const clearClipboardHistory = async (): Promise<void> => {
  try {
    hideDropdownMenu()
    // 添加确认对话框
    if (confirm('确定要清空所有剪贴板历史记录吗？此操作不可撤销。')) {
      await clipboard.clearHistory()
      // await loadClipboardHistory()
    }
  } catch (error) {
    console.error('[前端] 清空历史失败:', error)
  }
}

/** 关闭动画不等待搜索框收起或重新查询历史。 */
const closeWindow = async (): Promise<void> => {
  const id = transitionId
  if (closingTransitionId === id) return
  closingTransitionId = id
  preparedOpening = null
  const version = ++animationVersion
  searchAndNavManager.cancelPendingActions()
  ++historyRequestId
  pendingHistoryLoad = undefined
  const startedAt = performance.now()
  hideDropdownMenu()
  hideContextMenu()
  traceOverlay('closing')
  // 关闭一开始同步目标状态，紧接着的全局快捷键就能反向打开。
  send('overlay-close-started', id)
  if (!overlayVisible || animationManager.currentAnimationState.value === AnimationState.IDLE) {
    animationManager.resetOverlayState()
    traceOverlay('closed', { reason: 'already-hidden' })
    send('close-overlay-window', id)
    return
  }
  animationManager.setAnimationState(AnimationState.SLIDING_OUT)
  const animation = animationManager.createSlideOutAnimation()
  animation.play()
  const finished = await animation.finished
  if (!finished || version !== animationVersion || id !== transitionId) {
    traceOverlay('cancelled', { receivedId: id, reason: 'close-superseded' })
    return
  }
  traceOverlay('closed', {
    elapsedMs: Math.round(performance.now() - startedAt),
    reason: 'styles-hidden',
    transform: overlayContentRef.value
      ? getComputedStyle(overlayContentRef.value).transform
      : undefined
  })
  send('close-overlay-window', id)
}

/** 切换搜索框；收起时同时清除筛选。 */
const toggleSearch = async (): Promise<void> => {
  try {
    if (animationManager.isSearchVisible.value) {
      await Promise.all([searchAndNavManager.clearSearch(), hideSearchBox()])
    } else {
      await showSearchBox()
    }
  } catch (error) {
    console.error('[搜索框] 切换失败:', error)
    // 重置动画状态，防止卡死
    animationManager.setSearchAnimating(false)
    throw new OverlayError('搜索框切换动画失败', OverlayErrorCode.ANIMATION_FAILED, {
      originalError: error
    })
  }
}

/** 隐藏搜索框，输入控件仍可接收下一次输入。 */
const hideSearchBox = async (): Promise<void> => {
  const searchVersion = ++searchAnimationVersion
  animationManager.setSearchVisible(false)
  animationManager.setSearchAnimating(true)
  searchAndNavManager.focusSearchInput()

  const version = animationVersion
  const { play: playSearchCollapse, finished: searchCollapseFinished } =
    animationManager.createSearchCollapseAnimation()
  const { play: playTabsShow, finished: tabsShowFinished } =
    animationManager.createTabsShowAnimation()

  // 并行执行动画
  playSearchCollapse()
  playTabsShow()

  await Promise.all([searchCollapseFinished, tabsShowFinished])
  if (version !== animationVersion || searchVersion !== searchAnimationVersion) return

  animationManager.setSearchAnimating(false)
}

/** 展开只负责视觉变化，输入不等待动画完成。 */
const showSearchBox = async (): Promise<void> => {
  if (animationManager.isSearchVisible.value) return
  const searchVersion = ++searchAnimationVersion
  const version = animationVersion
  animationManager.setSearchAnimating(true)
  animationManager.setSearchVisible(true)

  await nextTick()
  if (version !== animationVersion || searchVersion !== searchAnimationVersion) return
  searchAndNavManager.focusSearchInput()

  const { play: playTabsHide, finished: tabsHideFinished } =
    animationManager.createTabsHideAnimation()
  const { play: playSearchExpand, finished: searchExpandFinished } =
    animationManager.createSearchExpandAnimation()

  // 并行执行动画
  playTabsHide()
  playSearchExpand()

  await Promise.all([tabsHideFinished, searchExpandFinished])
  if (version !== animationVersion || searchVersion !== searchAnimationVersion) return

  animationManager.setSearchAnimating(false)
}

// ==================== 搜索回调函数 ====================
const performSearchCallback = async (text: string): Promise<boolean> => {
  const filterKey = getHistoryFilterKey(text, getActiveTagIds())
  let promise =
    pendingHistoryLoad?.filterKey === filterKey
      ? pendingHistoryLoad.promise
      : loadClipboardHistory({
          searchText: text.trim(),
          tagIds: getActiveTagIds(),
          limit: 50,
          orderBy: 'last_used',
          orderDirection: 'DESC'
        })

  // 后台刷新可接替同一筛选的查询，确认操作继续等待最新请求。
  while (true) {
    const applied = await promise
    if (applied || filterKey !== getCurrentHistoryFilterKey() || !overlayVisible) return applied
    const latest = pendingHistoryLoad
    if (!latest || latest.filterKey !== filterKey || latest.promise === promise) return false
    promise = latest.promise
  }
}

// ==================== 初始化搜索和导航管理 ====================
const searchAndNavManager = useSearchAndNavigation({
  searchInput: searchInputRef,
  clipboardHistory,
  selectedIndex,
  isSearchVisible: animationManager.isSearchVisible,
  isActive: () => overlayVisible && closingTransitionId === null,
  areResultsCurrent: () => appliedHistoryFilterKey === getCurrentHistoryFilterKey(),
  performSearch: performSearchCallback,
  onRestoreClipboard: restoreClipboardContent,
  onOpenSearch: showSearchBox,
  onDismiss: () => handleCloseWindow()
})

// ==================== 初始化滚动和标签管理 ====================
scrollAndTagsManager = useScrollAndTags(
  clipboardScrollRef,
  clipboardHistory,
  selectedIndex,
  animationManager.currentAnimationState,
  async (options) => {
    await loadClipboardHistory(options)
  },
  () => searchAndNavManager.searchText.value || '',
  isSideLayout
)
const tabIndicatorStyle = useTabIndicator(tabsContainerRef, scrollAndTagsManager.tabs)

// ==================== 动画流程函数 ====================

const prepareOpening = async (position: OverlayPosition, id: number): Promise<void> => {
  if (id < transitionId) {
    traceOverlay('ignored', { receivedId: id, reason: 'stale-prepare' })
    return
  }
  retainPreviews()
  transitionId = id
  const version = ++animationVersion
  const startedAt = performance.now()
  overlayVisible = false
  searchAndNavManager.resetSearchState()
  ++historyRequestId
  pendingHistoryLoad = undefined
  closingTransitionId = null
  animationManager.resetOverlayState()
  applyOverlayPosition(position)
  traceOverlay('prepare')
  await nextTick()
  if (id !== transitionId || version !== animationVersion) {
    traceOverlay('cancelled', { receivedId: id, reason: 'prepare-superseded' })
    return
  }
  const element = overlayContentRef.value
  if (!element) throw new Error('Overlay animation target is unavailable')
  const animation = animationManager.createSlideInAnimation()
  animation.prepare()
  preparedOpening = { id, animation }
  // 隐藏窗口可能暂停动画帧；同步解析暂停的首帧样式，不能等绘制回调后再 show()。
  const transform = getComputedStyle(element).transform
  traceOverlay('prepared', {
    elapsedMs: Math.round(performance.now() - startedAt),
    transform,
    viewportWidth: window.innerWidth,
    viewportHeight: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio
  })
  send('overlay-open-prepared', id)
}

/**
 * 触发滑入动画
 */
const triggerSlideInAnimation = async (id: number): Promise<void> => {
  if (id < transitionId) {
    traceOverlay('ignored', { receivedId: id, reason: 'stale-open' })
    return
  }
  transitionId = id
  overlayVisible = true
  retainPreviews()
  closingTransitionId = null
  const version = ++animationVersion
  const startedAt = performance.now()
  const animation =
    preparedOpening?.id === id
      ? preparedOpening.animation
      : animationManager.createSlideInAnimation()
  preparedOpening = null
  animationManager.setAnimationState(AnimationState.SLIDING_IN)
  traceOverlay('opening')
  animation.play()
  searchAndNavManager.focusSearchInput()
  const finished = await animation.finished
  if (!finished || version !== animationVersion || id !== transitionId) {
    traceOverlay('cancelled', { receivedId: id, reason: 'open-superseded' })
    return
  }
  animationManager.setAnimationState(AnimationState.ACTIVE)
  traceOverlay('opened', { elapsedMs: Math.round(performance.now() - startedAt) })
  send('open-overlay-finished', id)
  if (historyDirty) throttledLoadClipboardHistory()
}

const handleCloseWindow = async (force = false, id = transitionId): Promise<void> => {
  if (id < transitionId) {
    traceOverlay('ignored', { receivedId: id, reason: 'stale-close' })
    return
  }
  transitionId = id
  if (force) {
    await closeWindow()
    return
  }
  if (animationManager.isSearchVisible.value) {
    if (searchAndNavManager.searchText.value) {
      searchAndNavManager.focusSearchInput()
      await searchAndNavManager.clearSearch()
    } else {
      await hideSearchBox()
    }
    return
  }
  await closeWindow()
}

// ==================== 快捷选择功能 ====================

/**
 * 处理键盘按下事件
 */
const handleKeyboardEvent = (event: KeyboardEvent): void => {
  // 输入法组词时，方向键、回车和 Escape 由输入法处理。
  if (
    event.defaultPrevented ||
    event.isComposing ||
    event.keyCode === 229 ||
    searchAndNavManager.isComposing.value
  )
    return

  // 聚焦按钮时保留 Enter / Space 的原生激活行为，避免误粘贴历史内容。
  if (
    event.target instanceof Element &&
    event.target.closest('button') &&
    ['Enter', ' '].includes(event.key)
  )
    return
  if (event.key === 'Escape' && isDropdownVisible.value) {
    event.preventDefault()
    hideDropdownMenu()
    return
  }
  const isCtrlOrCmd = event.ctrlKey || event.metaKey

  if (isCtrlOrCmd) {
    const numberKey = Number(event.key)

    if (!isNaN(numberKey) && numberKey >= 1 && numberKey <= 9) {
      event.preventDefault()
      if (!event.repeat) searchAndNavManager.restoreSelectedItem(numberKey - 1)
      return
    }

    if (!isQuickSelectMode.value) {
      isQuickSelectMode.value = true
    }
  }

  searchAndNavManager.handleKeyNavigation(event)
}

/**
 * 处理键盘释放事件
 */
const handleKeyboardUpEvent = (event: KeyboardEvent): void => {
  if (!event.ctrlKey && !event.metaKey && isQuickSelectMode.value) {
    isQuickSelectMode.value = false
  }
}

const handleTabsContainerWheel = (event: WheelEvent): boolean => {
  const target = event.target as HTMLElement | null
  const tabsElement = target?.closest('.tabs-container') as HTMLElement | null
  if (!tabsElement) {
    return false
  }

  const hasHorizontalOverflow = tabsElement.scrollWidth > tabsElement.clientWidth + 1
  if (!hasHorizontalOverflow) {
    return false
  }

  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
  if (Math.abs(delta) < 1) {
    return true
  }

  event.preventDefault()
  tabsElement.scrollLeft += delta
  return true
}

// ==================== 样式初始化 ====================

// ==================== 生命周期钩子 ====================

/**
 * 监听 selectedIndex 变化，自动滚动到可视区域
 */
watch(selectedIndex, () => {
  if (searchAndNavManager.isKeyboardNavigation.value) {
    nextTick(() => {
      scrollAndTagsManager.scrollToSelectedItem()
      searchAndNavManager.setKeyboardNavigation(false)
    })
  }
})

watch(isSideLayout, () => {
  nextTick(() => {
    scrollAndTagsManager.initializeScrollbar()
    scrollAndTagsManager.scrollToSelectedItem()
  })
})

/**
 * 组件挂载时的初始化逻辑
 */
onMounted(async () => {
  try {
    applyOverlayPosition(overlayPosition.value)
    ipcUnsubscribers.push(onSimple('trigger-close-window', handleCloseWindow))
    ipcUnsubscribers.push(
      onSimple('prepare-overlay-open', (position: OverlayPosition, id: number) => {
        prepareOpening(position, id).catch((error) => {
          traceOverlay('error', {
            receivedId: id,
            reason: `prepare-failed: ${error instanceof Error ? error.message : String(error)}`
          })
          console.error('[Overlay] 准备打开失败:', error)
          send('close-overlay-window', id)
        })
      })
    )
    ipcUnsubscribers.push(
      onSimple('overlay-hidden', (id: number) => {
        if (id < transitionId) {
          traceOverlay('ignored', { receivedId: id, reason: 'stale-hidden' })
          return
        }
        transitionId = id
        ++animationVersion
        overlayVisible = false
        closingTransitionId = null
        preparedOpening = null
        ++historyRequestId
        pendingHistoryLoad = undefined
        historyDirty = true
        throttledLoadClipboardHistory.cancel()
        isQuickSelectMode.value = false
        hideDropdownMenu()
        searchAndNavManager.resetSearchState()
        animationManager.resetOverlayState()
        schedulePreviewRelease()
        traceOverlay('hidden')
      })
    )
    ipcUnsubscribers.push(
      onSimple('trigger-slide-in-animation', (position: OverlayPosition, id: number) => {
        if (id < transitionId) {
          traceOverlay('ignored', { receivedId: id, reason: 'stale-open' })
          return
        }
        applyOverlayPosition(position)
        triggerSlideInAnimation(id).catch((error) => {
          traceOverlay('error', {
            receivedId: id,
            reason: `open-failed: ${error instanceof Error ? error.message : String(error)}`
          })
          console.error('[Overlay] 打开动画失败:', error)
          send('close-overlay-window', id)
        })
      })
    )
    const stopPositionWatch = watch(() => settingsStore.overlayPosition, applyOverlayPosition, {
      immediate: true
    })
    ipcUnsubscribers.push(stopPositionWatch)

    const removeOverlayPositionUpdatedListener = onSimple(
      'overlay-position-updated',
      (position?: OverlayPosition) => {
        applyOverlayPosition(position)
      }
    )
    ipcUnsubscribers.push(removeOverlayPositionUpdatedListener)

    eventManager.addEventListener(document, 'keydown', (event: Event) => {
      handleKeyboardEvent(event as KeyboardEvent)
    })

    eventManager.addEventListener(document, 'keyup', (event: Event) => {
      handleKeyboardUpEvent(event as KeyboardEvent)
    })

    eventManager.addEventListener(document, 'click', (event: Event) => {
      const target = event.target as HTMLElement
      const dropdownContainer = target.closest('.dropdown-container')

      if (!dropdownContainer && isDropdownVisible.value) {
        hideDropdownMenu()
      }
    })

    wheelHandler = (event: WheelEvent) => {
      // 详情弹窗使用原生滚动，不参与历史列表的横向/纵向滚动映射。
      if ((event.target as Element | null)?.closest('.clipboard-preview-dialog')) return
      if (handleTabsContainerWheel(event)) {
        return
      }

      scrollAndTagsManager.handleWheel(event)
    }
    document.addEventListener('wheel', wheelHandler, { passive: false, capture: true })

    animationManager.initializeStyles()

    const removeClipboardUpdateListener = onSimple(
      'clipboard-history-updated',
      throttledLoadClipboardHistory
    )
    ipcUnsubscribers.push(removeClipboardUpdateListener)

    const removeTagsUpdateListener = onSimple('tags-updated', () => {
      scrollAndTagsManager.reloadTags().catch(console.error)
    })
    ipcUnsubscribers.push(removeTagsUpdateListener)

    const removeOCRUpdateListener = onSimple(
      'record-ocr-updated',
      (data: { contentId: string; ocrText: string; timestamp: number }) => {
        const targetIndex = clipboardHistory.value.findIndex((item) => item.id === data.contentId)
        if (targetIndex >= 0 && clipboardHistory.value[targetIndex].content.ocr !== data.ocrText) {
          const targetItem = clipboardHistory.value[targetIndex]
          const updatedHistory = [...clipboardHistory.value]
          updatedHistory[targetIndex] = {
            ...targetItem,
            content: {
              ...targetItem.content,
              ocr: data.ocrText
            }
          }
          clipboardHistory.value = updatedHistory
        }
      }
    )
    ipcUnsubscribers.push(removeOCRUpdateListener)
    ipcUnsubscribers.push(
      onSimple('ocr:task-status', (status: OCRTaskStatus) => {
        if (status.state === 'completed' || status.state === 'cancelled') {
          removeOCRStatus(status.contentId)
          return
        }
        if (status.state === 'error') {
          if (!clipboardHistory.value.some((item) => item.id === status.contentId)) return
          ocrErrorsExpireAt.set(status.contentId, Date.now() + 2 * 60000)
        } else ocrErrorsExpireAt.delete(status.contentId)
        ocrStatuses.value[status.contentId] = status
        const ids = Object.keys(ocrStatuses.value)
        for (const id of ids.slice(0, Math.max(0, ids.length - 500))) removeOCRStatus(id)
      })
    )

    ocrExpiryTimer = setInterval(pruneOCRStatuses, 60000)
    schedulePreviewRelease()
    const preloadPromises = [
      loadClipboardHistory({
        tagIds: [],
        searchText: ''
      }).catch(() => []),
      scrollAndTagsManager.loadAllTags().catch(() => [])
    ]

    await Promise.allSettled(preloadPromises)

    send('overlay-ready')
  } catch (error) {
    console.error('[前端] Overlay 组件初始化失败:', error)
    throw new OverlayError('Overlay 组件初始化失败', OverlayErrorCode.INVALID_STATE, {
      originalError: error
    })
  }
})

/**
 * 组件卸载时的清理逻辑
 */
onUnmounted(() => {
  clearTimeout(previewIdleTimer)
  clearInterval(ocrExpiryTimer)
  ++animationVersion
  try {
    throttledLoadClipboardHistory.cancel()
    searchAndNavManager.cleanup()
    scrollAndTagsManager.destroyScrollbar()
    ipcUnsubscribers.forEach((unsubscribe) => unsubscribe())
    ipcUnsubscribers.length = 0

    ++historyRequestId
    if (wheelHandler) {
      document.removeEventListener('wheel', wheelHandler, { capture: true })
      wheelHandler = null
    }
  } catch (error) {
    console.error('[前端] Overlay 组件资源清理失败:', error)
  }
})
const isFilteredHistory = computed(
  () =>
    Boolean(searchAndNavManager.searchText.value.trim()) ||
    scrollAndTagsManager.activeTagId.value !== 'all'
)
</script>

<template>
  <div class="overlay-container" :class="`overlay-container--${overlayPosition}`">
    <div
      ref="overlayContentRef"
      :class="[
        'overlay-content',
        `overlay-content--${overlayPosition}`,
        { 'overlay-content--side': isSideLayout }
      ]"
    >
      <!-- 顶部工具栏 -->
      <div class="top-toolbar">
        <!-- 搜索图标 -->
        <button
          type="button"
          class="toolbar-icon search-icon"
          :title="OVERLAY_LABELS.search"
          :aria-label="OVERLAY_LABELS.search"
          :aria-expanded="animationManager.isSearchVisible.value"
          @click="toggleSearch"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <circle cx="11" cy="11" r="8"></circle>
            <path d="m21 21-4.35-4.35"></path>
          </svg>
        </button>

        <!-- 搜索输入框 -->
        <div
          ref="searchContainerRef"
          class="search-container"
          :class="{
            'search-container--collapsed': !animationManager.showSearchContainer.value
          }"
        >
          <input
            ref="searchInputRef"
            type="text"
            class="search-input"
            :placeholder="OVERLAY_LABELS.searchPlaceholder"
            :aria-label="OVERLAY_LABELS.search"
            spellcheck="false"
            autocomplete="off"
            :tabindex="animationManager.isSearchVisible.value ? 0 : -1"
            :value="searchAndNavManager.searchText.value"
            @input="searchAndNavManager.handleSearchInput"
            @compositionstart="searchAndNavManager.handleCompositionStart"
            @compositionend="searchAndNavManager.handleCompositionEnd"
          />
        </div>

        <!-- 标签组 -->
        <div
          v-show="animationManager.showTabsContainer.value"
          ref="tabsContainerRef"
          class="tabs-container"
        >
          <span class="tab-indicator" :style="tabIndicatorStyle" aria-hidden="true" />
          <button
            v-for="tab in scrollAndTagsManager.tabs.value"
            :key="tab.id"
            type="button"
            class="tab"
            :class="{
              active: tab.active,
              'has-count': tab.count > 0
            }"
            :aria-pressed="tab.active"
            :aria-busy="scrollAndTagsManager.isTagSwitching.value && tab.active"
            :aria-label="`${tab.label}${tab.count > 0 ? ` (${tab.count})` : ''}`"
            @click="scrollAndTagsManager.switchTag(tab.id)"
          >
            <span class="tab-dot" :style="{ backgroundColor: tab.color }"></span>
            <span class="tab-label">{{ tab.label }}</span>
            <span v-if="tab.count > 0" class="tab-count">{{ tab.count }}</span>
            <!-- 加载指示器 -->
            <div v-if="scrollAndTagsManager.isTagSwitching.value && tab.active" class="tab-loading">
              <div class="loading-spinner"></div>
            </div>
          </button>
        </div>

        <!-- 右侧按钮组 -->
        <div class="toolbar-buttons">
          <div class="dropdown-container">
            <button
              type="button"
              class="toolbar-icon menu-icon"
              :title="OVERLAY_LABELS.menu"
              :aria-label="OVERLAY_LABELS.menu"
              :aria-expanded="isDropdownVisible"
              @click="toggleDropdownMenu"
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
              >
                <circle cx="12" cy="12" r="1"></circle>
                <circle cx="19" cy="12" r="1"></circle>
                <circle cx="5" cy="12" r="1"></circle>
              </svg>
            </button>

            <!-- 下拉菜单 -->
            <div v-if="isDropdownVisible" class="dropdown-menu">
              <button type="button" class="dropdown-item" @click="openSettings">
                <span class="dropdown-icon">
                  <IconSettings />
                </span>
                <span class="dropdown-label">设置</span>
              </button>
              <button
                type="button"
                class="dropdown-item dropdown-item--danger"
                @click="clearClipboardHistory"
              >
                <span class="dropdown-icon">
                  <IconDelete />
                </span>
                <span class="dropdown-label">清空历史</span>
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="overlay-body">
        <!-- 空状态 -->
        <div v-if="clipboardHistory.length === 0" class="empty-container">
          <div class="empty-icon" aria-hidden="true"><IconClipboard /></div>
          <p class="empty-text">
            {{ isFilteredHistory ? OVERLAY_LABELS.noResults : OVERLAY_LABELS.empty }}
          </p>
          <p class="empty-hint">
            {{ isFilteredHistory ? OVERLAY_LABELS.noResultsHint : OVERLAY_LABELS.emptyHint }}
          </p>
        </div>

        <!-- 剪贴板历史横向滚动 -->
        <div
          v-else
          ref="clipboardScrollRef"
          :class="[
            'clipboard-scroll-container',
            { 'clipboard-scroll-container--side': isSideLayout }
          ]"
        >
          <transition-group
            name="clipboard-list"
            tag="div"
            :class="['clipboard-list-wrapper', { 'clipboard-list-wrapper--side': isSideLayout }]"
          >
            <ClipboardCard
              v-for="(item, index) in clipboardHistory"
              :key="item.id"
              v-memo="[
                item,
                previewsActive,
                ocrStatuses[item.id],
                index,
                index === selectedIndex,
                isQuickSelectMode,
                scrollAndTagsManager.allTags.value,
                scrollAndTagsManager.activeTagId.value,
                item.source_app ? clipboardApps[item.source_app] : undefined
              ]"
              :previews-active="previewsActive"
              :item="item"
              :ocr-status="ocrStatuses[item.id]"
              :index="index"
              :is-selected="index === selectedIndex"
              :is-quick-select-mode="isQuickSelectMode"
              :all-tags="scrollAndTagsManager.allTags.value"
              :active-tag-id="scrollAndTagsManager.activeTagId.value"
              :apps="clipboardApps"
              @click="restoreClipboardContent(item.id)"
              @restore-as-text="restoreClipboardContentAsText(item.id)"
              @restore-ocr-text="restoreClipboardContentAsOcrText(item.id)"
              @parse-ocr="parseOcrFromClipboardItem(item.id)"
              @parse-q-r-code="parseQRCodeFromClipboardItem(item.id)"
              @generate-q-r-code="generateQRCodeFromClipboardItem(item.id)"
              @delete="deleteClipboardItem(item.id)"
            />
          </transition-group>
        </div>
      </div>
    </div>
  </div>

  <!-- 全局右键菜单 -->
  <ContextMenu />
</template>

<style scoped>
@import url('./index.css');
</style>
