/**
 * 滚动和标签管理 Composable
 * 合并滚动管理和标签管理，提供统一的数据和交互接口
 */

import { ref, computed, nextTick, onUnmounted, readonly, type Ref } from 'vue'
import type { ClipboardItem, TagGroup, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import { tags } from '@renderer/api/renderer-api'
import { useRafThrottle, useStateManager } from './useUtils'
import { AnimationState } from './useAnimation'
import { useContextMenu } from './useContextMenu'
import Scrollbar from 'smooth-scrollbar'
import type { ScrollbarOptions } from 'smooth-scrollbar/interfaces'

// ==================== 类型定义 ====================

export interface TabData {
  id: string
  label: string
  active: boolean
  color: string
  count: number
}

export interface TagState {
  allTags: TagGroup[]
  activeTagId: string
  isTagSwitching: boolean
  tagStats: Record<string, number>
}

export interface ScrollAndTagsManager {
  // 滚动状态
  readonly isScrolling: Ref<boolean>

  // 标签状态
  readonly allTags: Ref<TagGroup[]>
  readonly activeTagId: Ref<string>
  readonly isTagSwitching: Ref<boolean>
  readonly tagStats: Ref<Record<string, number>>
  readonly tabs: Ref<TabData[]>

  // 滚动方法
  scrollToSelectedItem: () => void
  handleWheel: (event: WheelEvent) => void
  ensureSelectedItemVisible: () => void
  initializeScrollbar: () => void
  destroyScrollbar: () => void

  // 标签方法
  loadAllTags: () => Promise<void>
  loadTagStats: () => Promise<void>
  switchTag: (tagId: string) => Promise<void>
  reloadTags: () => Promise<void>
}

// ==================== 常量定义 ====================

const SCROLL_MARGIN = 40 // 滚动时的边距，确保选中项不会紧贴边缘
const WHEEL_LINE_HEIGHT = 28

const SCROLLBAR_OPTIONS: Partial<ScrollbarOptions> = {
  damping: 0.1,
  renderByPixels: false,
  alwaysShowTracks: false,
  continuousScrolling: true
}

// ==================== 主要函数 ====================

export function useScrollAndTags(
  clipboardScrollRef: Ref<HTMLElement | undefined>,
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  currentAnimationState: Ref<AnimationState>,
  loadClipboardHistory: (options?: ClipboardHistoryQueryOptions) => Promise<void>,
  getSearchText: () => string,
  isVerticalLayout: Ref<boolean>
): ScrollAndTagsManager {
  // ==================== 状态管理 ====================

  const isScrolling = ref(false)
  let scrollbarInstance: Scrollbar | null = null
  let lastWheelDirection = 0

  // 获取上下文菜单管理器
  const { hideContextMenu } = useContextMenu()

  const tagStateManager = useStateManager<TagState>({
    allTags: [],
    activeTagId: 'all',
    isTagSwitching: false,
    tagStats: {}
  })

  // 提取状态引用
  const allTags = computed<TagGroup[]>(() => tagStateManager.state.value.allTags)
  const activeTagId = computed<string>(() => tagStateManager.state.value.activeTagId)
  const isTagSwitching = computed<boolean>(() => tagStateManager.state.value.isTagSwitching)
  const tagStats = computed<Record<string, number>>(() => tagStateManager.state.value.tagStats)

  // 计算标签组件数据
  const tabs = computed<TabData[]>(() => {
    const allTab: TabData = {
      id: 'all',
      label: 'All',
      active: activeTagId.value === 'all',
      color: '#10b981',
      count: 0
    }

    const tagTabs: TabData[] = allTags.value.map((tag) => ({
      id: tag.id,
      label: tag.name,
      active: activeTagId.value === tag.id,
      color: tag.color,
      count: tagStats.value[tag.id] || 0
    }))

    return [allTab, ...tagTabs]
  })

  // ==================== 滚动管理 ====================

  /**
   * 初始化 smooth-scrollbar 实例
   * 注意：滚动条隐藏样式已在 overlay.vue 的 CSS 中直接定义
   */
  const initializeScrollbar = (): void => {
    if (
      scrollbarInstance &&
      scrollbarInstance.containerEl === clipboardScrollRef.value &&
      clipboardScrollRef.value?.isConnected
    ) {
      scrollbarInstance.update()
      return
    }

    lastWheelDirection = 0
    // 容器被 v-if 替换后才重建；普通数据更新保留滚动位置。
    // 如果已有实例，先销毁再重新创建
    if (scrollbarInstance) {
      try {
        scrollbarInstance.destroy()
        scrollbarInstance = null
      } catch (error) {
        console.warn('[滚动标签管理] 销毁旧的 scrollbar 实例时出错:', error)
        scrollbarInstance = null
      }
    }

    if (!clipboardScrollRef.value) return
    try {
      // 仅初始化 scrollbar 实例，样式在 overlay.vue 的 CSS 中定义
      scrollbarInstance = Scrollbar.init(clipboardScrollRef.value, SCROLLBAR_OPTIONS)
    } catch (error) {
      console.error('[滚动标签管理] smooth-scrollbar 初始化失败:', error)
      scrollbarInstance = null
    }
  }

  const destroyScrollbar = (): void => {
    lastWheelDirection = 0
    if (scrollbarInstance) {
      try {
        scrollbarInstance.destroy()
        scrollbarInstance = null
      } catch (error) {
        console.error('[滚动标签管理] 销毁 smooth-scrollbar 实例失败:', error)
      }
    }
  }

  const getSelectedCardElement = (): HTMLElement | null => {
    if (!clipboardScrollRef.value || selectedIndex.value < 0) {
      return null
    }

    return clipboardScrollRef.value.querySelector<HTMLElement>('.clipboard-card.selected')
  }

  const getScrollMetrics = (): {
    scrollLeft: number
    scrollTop: number
    containerWidth: number
    containerHeight: number
    contentWidth: number
    contentHeight: number
  } | null => {
    if (scrollbarInstance) {
      return {
        scrollLeft: scrollbarInstance.scrollLeft,
        scrollTop: scrollbarInstance.scrollTop,
        containerWidth: scrollbarInstance.size.container.width,
        containerHeight: scrollbarInstance.size.container.height,
        contentWidth: scrollbarInstance.size.content.width,
        contentHeight: scrollbarInstance.size.content.height
      }
    }

    if (!clipboardScrollRef.value) {
      return null
    }

    return {
      scrollLeft: clipboardScrollRef.value.scrollLeft,
      scrollTop: clipboardScrollRef.value.scrollTop,
      containerWidth: clipboardScrollRef.value.clientWidth,
      containerHeight: clipboardScrollRef.value.clientHeight,
      contentWidth: clipboardScrollRef.value.scrollWidth,
      contentHeight: clipboardScrollRef.value.scrollHeight
    }
  }

  const scrollToAxisPosition = (target: number, axis: 'x' | 'y'): void => {
    hideContextMenu()

    if (scrollbarInstance) {
      try {
        const targetX = axis === 'x' ? target : scrollbarInstance.scrollLeft
        const targetY = axis === 'y' ? target : scrollbarInstance.scrollTop
        scrollbarInstance.setMomentum(0, 0)
        lastWheelDirection = 0
        scrollbarInstance.scrollTo(targetX, targetY, 300)
        return
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 滚动失败，回退到原生滚动:', error)
      }
    }

    if (!clipboardScrollRef.value) {
      return
    }

    clipboardScrollRef.value.scrollTo({
      left: axis === 'x' ? target : clipboardScrollRef.value.scrollLeft,
      top: axis === 'y' ? target : clipboardScrollRef.value.scrollTop,
      behavior: 'smooth'
    })
  }

  const scrollToSelectedItem = useRafThrottle(() => {
    if (!clipboardScrollRef.value || clipboardHistory.value.length === 0 || selectedIndex.value < 0)
      return

    // 检查 scrollbar 实例是否有效，如果无效则重新初始化
    if (
      !scrollbarInstance ||
      !scrollbarInstance.containerEl ||
      !scrollbarInstance.containerEl.parentNode
    ) {
      initializeScrollbar()
    }
    const selectedCardElement = getSelectedCardElement()
    const metrics = getScrollMetrics()
    if (!selectedCardElement || !metrics) {
      return
    }

    const verticalLayout = isVerticalLayout.value
    const containerStart = verticalLayout ? metrics.scrollTop : metrics.scrollLeft
    const containerSize = verticalLayout ? metrics.containerHeight : metrics.containerWidth
    const contentSize = verticalLayout ? metrics.contentHeight : metrics.contentWidth
    const maxScroll = Math.max(0, contentSize - containerSize)

    const selectedStart = verticalLayout
      ? selectedCardElement.offsetTop
      : selectedCardElement.offsetLeft
    const selectedSize = verticalLayout
      ? selectedCardElement.offsetHeight
      : selectedCardElement.offsetWidth
    const selectedEnd = selectedStart + selectedSize
    const containerEnd = containerStart + containerSize

    let targetScroll: number | null = null

    if (selectedStart < containerStart + SCROLL_MARGIN) {
      targetScroll = Math.max(0, selectedStart - SCROLL_MARGIN)
    } else if (selectedEnd > containerEnd - SCROLL_MARGIN) {
      targetScroll = Math.min(maxScroll, selectedEnd - containerSize + SCROLL_MARGIN)
    }

    if (targetScroll !== null && Math.abs(targetScroll - containerStart) > 1) {
      scrollToAxisPosition(targetScroll, verticalLayout ? 'y' : 'x')
    }
  })

  // 辅助方法：检查选中项是否在可视区域内
  const isSelectedItemVisible = (): boolean => {
    if (
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0 ||
      selectedIndex.value < 0
    ) {
      return false
    }

    const selectedCardElement = getSelectedCardElement()
    const metrics = getScrollMetrics()
    if (!selectedCardElement || !metrics) {
      return false
    }

    const verticalLayout = isVerticalLayout.value
    const containerStart = verticalLayout ? metrics.scrollTop : metrics.scrollLeft
    const containerSize = verticalLayout ? metrics.containerHeight : metrics.containerWidth
    const containerEnd = containerStart + containerSize

    const selectedStart = verticalLayout
      ? selectedCardElement.offsetTop
      : selectedCardElement.offsetLeft
    const selectedSize = verticalLayout
      ? selectedCardElement.offsetHeight
      : selectedCardElement.offsetWidth
    const selectedEnd = selectedStart + selectedSize

    // 检查是否完全在可视区域内（包含边距）
    return (
      selectedStart >= containerStart + SCROLL_MARGIN && selectedEnd <= containerEnd - SCROLL_MARGIN
    )
  }

  const handleWheel = (event: WheelEvent): void => {
    const container = clipboardScrollRef.value
    if (!container || clipboardHistory.value.length === 0 || event.defaultPrevented) {
      return
    }

    // 捕获阶段接管事件，避免 smooth-scrollbar 的容器监听器再次叠加同一次滚动。
    event.preventDefault()
    event.stopPropagation()
    if (currentAnimationState.value !== AnimationState.ACTIVE) return

    const verticalLayout = isVerticalLayout.value
    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
    const deltaScale =
      event.deltaMode === WheelEvent.DOM_DELTA_LINE
        ? WHEEL_LINE_HEIGHT
        : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
          ? verticalLayout
            ? container.clientHeight
            : container.clientWidth
          : 1
    const scrollAmount = delta * deltaScale
    if (scrollAmount === 0) return

    hideContextMenu()

    // 检查 scrollbar 实例是否有效，如果无效则重新初始化
    if (
      !scrollbarInstance ||
      !scrollbarInstance.containerEl ||
      !scrollbarInstance.containerEl.parentNode
    ) {
      initializeScrollbar()
    }

    if (scrollbarInstance) {
      try {
        // 取消键盘定位的滚动动画，当前位置保持不变，后续位移交给库逐帧衰减。
        scrollbarInstance.scrollTo(scrollbarInstance.scrollLeft, scrollbarInstance.scrollTop, 0)
        const direction = Math.sign(scrollAmount)
        if (direction !== lastWheelDirection) {
          scrollbarInstance.setMomentum(0, 0)
        }
        lastWheelDirection = direction
        scrollbarInstance.addMomentum(
          verticalLayout ? 0 : scrollAmount,
          verticalLayout ? scrollAmount : 0
        )
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 实例操作失败，尝试重新初始化:', error)
        initializeScrollbar()
      }
    } else {
      container.scrollBy({
        left: verticalLayout ? 0 : scrollAmount,
        top: verticalLayout ? scrollAmount : 0,
        behavior: 'smooth'
      })
    }
  }

  const ensureSelectedItemVisible = (): void => {
    nextTick(() => {
      // 只有在选中项不可见时才滚动
      if (!isSelectedItemVisible()) {
        scrollToSelectedItem()
      }
    })
  }

  // ==================== 标签管理 ====================

  const loadAllTags = async (): Promise<void> => {
    try {
      const tagList = await tags.getAllTags()
      tagStateManager.setState((prev) => ({
        ...prev,
        allTags: tagList
      }))

      // 加载标签统计数据
      await loadTagStats()
    } catch (error) {
      console.error('[滚动标签管理] 加载标签数据失败:', error)
      tagStateManager.setState((prev) => ({
        ...prev,
        allTags: []
      }))
    }
  }

  const loadTagStats = async (): Promise<void> => {
    try {
      // 从后端获取标签统计数据
      const statsArray = await tags.getTagStats()
      const stats: Record<string, number> = {}

      // 将数组格式转换为对象格式，方便查找
      for (const stat of statsArray) {
        stats[stat.tagId] = stat.recordCount
      }

      tagStateManager.setState((prev) => ({
        ...prev,
        tagStats: stats
      }))
    } catch (error) {
      console.error('[滚动标签管理] 加载标签统计数据失败:', error)
      tagStateManager.setState((prev) => ({
        ...prev,
        tagStats: {}
      }))
    }
  }

  const switchTag = async (tagId: string): Promise<void> => {
    if (activeTagId.value === tagId || isTagSwitching.value) return

    tagStateManager.setState((prev) => ({
      ...prev,
      isTagSwitching: true,
      activeTagId: tagId
    }))

    try {
      const currentTagIds = tagId === 'all' ? [] : [tagId]
      const currentSearchText = getSearchText()

      await loadClipboardHistory({
        tagIds: currentTagIds,
        searchText: currentSearchText,
        limit: 50,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })
    } catch (error) {
      console.error('[滚动标签管理] 切换标签失败:', error)
    } finally {
      tagStateManager.setState((prev) => ({
        ...prev,
        isTagSwitching: false
      }))
    }
  }

  const reloadTags = async (): Promise<void> => {
    try {
      await loadAllTags()
    } catch (error) {
      console.warn('[滚动标签管理] 重新加载标签数据失败:', error)
    }
  }

  // ==================== 清理 ====================

  onUnmounted(() => {
    destroyScrollbar()
  })

  // ==================== 返回接口 ====================

  return {
    // 滚动状态
    isScrolling: readonly(isScrolling),

    // 标签状态
    allTags,
    activeTagId: readonly(activeTagId),
    isTagSwitching: readonly(isTagSwitching),
    tagStats: readonly(tagStats),
    tabs,

    // 滚动方法
    scrollToSelectedItem,
    handleWheel,
    ensureSelectedItemVisible,
    initializeScrollbar,
    destroyScrollbar,

    // 标签方法
    loadAllTags,
    loadTagStats,
    switchTag,
    reloadTags
  }
}
