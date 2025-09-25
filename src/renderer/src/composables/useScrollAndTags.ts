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

const CARD_WIDTH = 300
const CARD_GAP = 16 // 修正为实际的 CSS gap 值
const CONTAINER_PADDING_LEFT = 20 // 容器左侧 padding
const TOTAL_CARD_WIDTH = CARD_WIDTH + CARD_GAP
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
  getSearchText: () => string
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
  const allTags = computed(() => tagStateManager.state.value.allTags)
  const activeTagId = computed(() => tagStateManager.state.value.activeTagId)
  const isTagSwitching = computed(() => tagStateManager.state.value.isTagSwitching)
  const tagStats = computed(() => tagStateManager.state.value.tagStats)

  // 计算标签组件数据
  const tabs = computed(() => {
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
    if (!clipboardScrollRef.value) return

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

    // 计算选中卡片的实际位置（考虑容器的 padding-left）
    const selectedCardLeft = CONTAINER_PADDING_LEFT + selectedIndex.value * TOTAL_CARD_WIDTH
    const selectedCardRight = selectedCardLeft + CARD_WIDTH

    let containerScrollLeft: number
    let containerWidth: number
    let maxScrollLeft: number

    if (scrollbarInstance) {
      containerScrollLeft = scrollbarInstance.scrollLeft
      containerWidth = scrollbarInstance.size.container.width
      maxScrollLeft = Math.max(
        0,
        scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
      )
    } else {
      const container = clipboardScrollRef.value
      containerScrollLeft = container.scrollLeft
      containerWidth = container.clientWidth
      maxScrollLeft = Math.max(0, container.scrollWidth - container.clientWidth)
    }

    const containerScrollRight = containerScrollLeft + containerWidth

    // 检查选中的卡片是否在可视区域内，并添加适当的边距
    let targetScrollLeft: number | null = null

    if (selectedCardLeft < containerScrollLeft + SCROLL_MARGIN) {
      // 选中卡片在左侧外部或太靠近左边缘
      targetScrollLeft = Math.max(0, selectedCardLeft - SCROLL_MARGIN)
    } else if (selectedCardRight > containerScrollRight - SCROLL_MARGIN) {
      // 选中卡片在右侧外部或太靠近右边缘
      targetScrollLeft = Math.min(maxScrollLeft, selectedCardRight - containerWidth + SCROLL_MARGIN)
    }

    // 只有在需要滚动时才执行滚动
    if (targetScrollLeft !== null && Math.abs(targetScrollLeft - containerScrollLeft) > 1) {
      scrollToPosition(targetScrollLeft)
      // console.log('[滚动标签管理] 滚动到选中项:', {
      //   selectedIndex: selectedIndex.value,
      //   selectedCardLeft,
      //   selectedCardRight,
      //   containerScrollLeft,
      //   containerScrollRight,
      //   targetScrollLeft,
      //   containerWidth
      // })
    }
  })

  const scrollToPosition = (targetLeft: number) => {
    // 程序化滚动时也隐藏菜单
    hideContextMenu()

    if (scrollbarInstance) {
      try {
        // 使用 smooth-scrollbar 的平滑滚动，动画时间设为 300ms
        scrollbarInstance.scrollTo(targetLeft, 0, 300)
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 滚动失败，回退到原生滚动:', error)
        // 如果 scrollbar 操作失败，回退到原生滚动
        if (clipboardScrollRef.value) {
          clipboardScrollRef.value.scrollTo({
            left: targetLeft,
            behavior: 'smooth'
          })
        }
      }
    } else if (clipboardScrollRef.value) {
      // 回退到原生滚动
      clipboardScrollRef.value.scrollTo({
        left: targetLeft,
        behavior: 'smooth'
      })
    }
  }

  // 辅助方法：检查选中项是否在可视区域内
  const isSelectedItemVisible = (): boolean => {
    if (
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0 ||
      selectedIndex.value < 0
    ) {
      return false
    }

    const selectedCardLeft = CONTAINER_PADDING_LEFT + selectedIndex.value * TOTAL_CARD_WIDTH
    const selectedCardRight = selectedCardLeft + CARD_WIDTH

    let containerScrollLeft: number
    let containerWidth: number

    if (scrollbarInstance) {
      containerScrollLeft = scrollbarInstance.scrollLeft
      containerWidth = scrollbarInstance.size.container.width
    } else {
      const container = clipboardScrollRef.value
      containerScrollLeft = container.scrollLeft
      containerWidth = container.clientWidth
    }

    const containerScrollRight = containerScrollLeft + containerWidth

    // 检查是否完全在可视区域内（包含边距）
    return (
      selectedCardLeft >= containerScrollLeft + SCROLL_MARGIN &&
      selectedCardRight <= containerScrollRight - SCROLL_MARGIN
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

    // 检查是否为水平滚动
    const isHorizontalScroll = Math.abs(event.deltaX) > Math.abs(event.deltaY)

    let scrollAmount = 0
    if (isHorizontalScroll && Math.abs(event.deltaX) > 3) {
      scrollAmount = event.deltaX * 3
    } else {
      scrollAmount = event.deltaY * 5
    }

    if (scrollbarInstance) {
      try {
        const currentScrollLeft = scrollbarInstance.scrollLeft
        const maxScrollLeft =
          scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
        const targetScrollLeft = Math.max(
          0,
          Math.min(maxScrollLeft, currentScrollLeft + scrollAmount)
        )

        if (targetScrollLeft !== currentScrollLeft) {
          scrollbarInstance.scrollTo(targetScrollLeft, 0, 0)
        }
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 实例操作失败，尝试重新初始化:', error)
        initializeScrollbar()
      }
    } else if (clipboardScrollRef.value) {
      const container = clipboardScrollRef.value
      const currentScrollLeft = container.scrollLeft
      const maxScrollLeft = container.scrollWidth - container.clientWidth
      const targetScrollLeft = Math.max(
        0,
        Math.min(maxScrollLeft, currentScrollLeft + scrollAmount)
      )

      if (targetScrollLeft !== currentScrollLeft) {
        container.scrollTo({
          left: targetScrollLeft,
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

      // 标签切换完成后，确保 scrollbar 正确初始化
      nextTick(() => {
        initializeScrollbar()
        scrollToSelectedItem()
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
    allTags: allTags as unknown as Ref<TagGroup[]>,
    activeTagId: readonly(activeTagId),
    isTagSwitching: readonly(isTagSwitching),
    tagStats: readonly(tagStats),
    tabs: tabs as unknown as Ref<TabData[]>,

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
