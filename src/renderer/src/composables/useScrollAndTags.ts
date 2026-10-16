/**
 * 滚动和标签管理 Composable
 * 合并滚动管理和标签管理，提供统一的数据和交互接口
 */

import { ref, computed, nextTick, onUnmounted, readonly, type Ref } from 'vue'
import type { ClipboardItem, TagGroup, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import { tags } from '@renderer/api/renderer-api'
import { useRafThrottle, useThrottle, useStateManager } from './useUtils'
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

const SCROLLBAR_OPTIONS: Partial<ScrollbarOptions> = {
  damping: 0.1,
  renderByPixels: true,
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
  const initializeScrollbar = () => {
    if (
      scrollbarInstance &&
      scrollbarInstance.containerEl === clipboardScrollRef.value &&
      clipboardScrollRef.value?.isConnected
    ) {
      scrollbarInstance.update()
      return
    }

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
      // console.log('[滚动标签管理] smooth-scrollbar 初始化成功')
    } catch (error) {
      console.error('[滚动标签管理] smooth-scrollbar 初始化失败:', error)
      scrollbarInstance = null
    }
  }

  const destroyScrollbar = () => {
    if (scrollbarInstance) {
      try {
        scrollbarInstance.destroy()
        scrollbarInstance = null
        // console.log('[滚动标签管理] smooth-scrollbar 实例已销毁')
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

  const getScrollMetrics = () => {
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

  const scrollToAxisPosition = (target: number, axis: 'x' | 'y') => {
    hideContextMenu()

    if (scrollbarInstance) {
      try {
        const targetX = axis === 'x' ? target : scrollbarInstance.scrollLeft
        const targetY = axis === 'y' ? target : scrollbarInstance.scrollTop
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

  const handleWheelThrottled = useThrottle((event: WheelEvent) => {
    // console.log('[滚动标签管理] 收到滚轮事件:', {
    //   animationState: currentAnimationState.value,
    //   hasScrollRef: !!clipboardScrollRef.value,
    //   historyLength: clipboardHistory.value.length,
    //   deltaX: event.deltaX,
    //   deltaY: event.deltaY
    // })

    // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
    if (
      currentAnimationState.value !== AnimationState.ACTIVE ||
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0
    ) {
      // console.log('[滚动标签管理] 滚轮事件被忽略，条件不满足')
      return
    }

    // 检查 scrollbar 实例是否有效，如果无效则重新初始化
    if (
      !scrollbarInstance ||
      !scrollbarInstance.containerEl ||
      !scrollbarInstance.containerEl.parentNode
    ) {
      initializeScrollbar()
    }

    event.preventDefault()

    const verticalLayout = isVerticalLayout.value
    const primaryDelta = verticalLayout ? event.deltaY : event.deltaX
    const fallbackDelta = verticalLayout ? event.deltaX : event.deltaY
    const dominantDelta =
      Math.abs(primaryDelta) > 3 ? primaryDelta : Math.abs(fallbackDelta) > 3 ? fallbackDelta : 0
    const scrollAmount = dominantDelta * (verticalLayout ? 2.5 : 3.5)

    if (scrollbarInstance) {
      try {
        const currentScroll = verticalLayout
          ? scrollbarInstance.scrollTop
          : scrollbarInstance.scrollLeft
        const maxScroll =
          (verticalLayout
            ? scrollbarInstance.size.content.height - scrollbarInstance.size.container.height
            : scrollbarInstance.size.content.width - scrollbarInstance.size.container.width) || 0
        const targetScroll = Math.max(0, Math.min(maxScroll, currentScroll + scrollAmount))

        if (targetScroll !== currentScroll) {
          const targetX = verticalLayout ? scrollbarInstance.scrollLeft : targetScroll
          const targetY = verticalLayout ? targetScroll : scrollbarInstance.scrollTop
          scrollbarInstance.scrollTo(targetX, targetY, 0)
        }
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 实例操作失败，尝试重新初始化:', error)
        initializeScrollbar()
      }
    } else if (clipboardScrollRef.value) {
      const container = clipboardScrollRef.value
      const currentScroll = verticalLayout ? container.scrollTop : container.scrollLeft
      const maxScroll = verticalLayout
        ? container.scrollHeight - container.clientHeight
        : container.scrollWidth - container.clientWidth
      const targetScroll = Math.max(0, Math.min(maxScroll, currentScroll + scrollAmount))

      if (targetScroll !== currentScroll) {
        container.scrollTo({
          left: verticalLayout ? container.scrollLeft : targetScroll,
          top: verticalLayout ? targetScroll : container.scrollTop,
          behavior: 'smooth'
        })
      }
    }
  }, 16)

  // 创建实际的 handleWheel 函数
  const handleWheel = (event: WheelEvent) => {
    // 滚动时隐藏所有菜单
    hideContextMenu()
    handleWheelThrottled(event)
  }

  const ensureSelectedItemVisible = () => {
    nextTick(() => {
      // 只有在选中项不可见时才滚动
      if (!isSelectedItemVisible()) {
        scrollToSelectedItem()
      }
    })
  }

  // ==================== 标签管理 ====================

  const loadAllTags = async () => {
    try {
      const tagList = await tags.getAllTags()
      tagStateManager.setState((prev) => ({
        ...prev,
        allTags: tagList
      }))
      // console.log('[滚动标签管理] 标签数据加载完成:', tagList)

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

  const loadTagStats = async () => {
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
      // console.log('[滚动标签管理] 标签统计数据加载完成:', stats)
    } catch (error) {
      console.error('[滚动标签管理] 加载标签统计数据失败:', error)
      tagStateManager.setState((prev) => ({
        ...prev,
        tagStats: {}
      }))
    }
  }

  const switchTag = async (tagId: string) => {
    if (activeTagId.value === tagId || isTagSwitching.value) return

    tagStateManager.setState((prev) => ({
      ...prev,
      isTagSwitching: true,
      activeTagId: tagId
    }))

    // console.log('[滚动标签管理] 切换到标签:', tagId)

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

  const reloadTags = async () => {
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
