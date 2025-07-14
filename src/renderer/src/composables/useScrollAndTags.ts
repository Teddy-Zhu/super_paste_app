/**
 * 滚动和标签管理 Composable
 * 合并滚动管理和标签管理，提供统一的数据和交互接口
 */

import { ref, computed, nextTick, onUnmounted, readonly, type Ref } from 'vue'
import type { ClipboardItem, TagGroup, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import { tags } from '@renderer/api/renderer-api'
import { useRafThrottle, useThrottle, useStateManager } from './useUtils'
import { AnimationState } from './useAnimation'
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
const CARD_GAP = 32
const TOTAL_CARD_WIDTH = CARD_WIDTH + CARD_GAP

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
  loadClipboardHistory: (options?: ClipboardHistoryQueryOptions) => Promise<void>
): ScrollAndTagsManager {
  
  // ==================== 状态管理 ====================
  
  const isScrolling = ref(false)
  let scrollbarInstance: Scrollbar | null = null
  
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
    
    const tagTabs: TabData[] = allTags.value.map(tag => ({
      id: tag.id,
      label: tag.name,
      active: activeTagId.value === tag.id,
      color: tag.color,
      count: tagStats.value[tag.id] || 0
    }))
    
    return [allTab, ...tagTabs]
  })
  
  // ==================== 滚动管理 ====================
  
  const initializeScrollbar = () => {
    if (!clipboardScrollRef.value || scrollbarInstance) return
    
    try {
      scrollbarInstance = Scrollbar.init(clipboardScrollRef.value, SCROLLBAR_OPTIONS)
      
      if (scrollbarInstance) {
        // 隐藏滚动条
        const scrollbarElement = scrollbarInstance.containerEl
        const style = scrollbarElement.style
        style.setProperty('--scrollbar-track-color', 'transparent')
        style.setProperty('--scrollbar-thumb-color', 'transparent')
        
        // 添加自定义样式来完全隐藏滚动条
        if (!document.getElementById('scrollbar-hide-style')) {
          const styleSheet = document.createElement('style')
          styleSheet.id = 'scrollbar-hide-style'
          styleSheet.textContent = `
            .scrollbar-track { display: none !important; }
            .scrollbar-thumb { display: none !important; }
          `
          document.head.appendChild(styleSheet)
        }
      }
      
      console.log('[滚动标签管理] smooth-scrollbar 初始化成功')
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
        console.log('[滚动标签管理] smooth-scrollbar 实例已销毁')
      } catch (error) {
        console.error('[滚动标签管理] 销毁 smooth-scrollbar 实例失败:', error)
      }
    }
  }
  
  const scrollToSelectedItem = useRafThrottle(() => {
    if (!clipboardScrollRef.value || clipboardHistory.value.length === 0) return
    
    // 如果 smooth-scrollbar 未初始化，先初始化
    if (!scrollbarInstance) {
      initializeScrollbar()
    }
    
    const selectedCardLeft = selectedIndex.value * TOTAL_CARD_WIDTH
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
    
    // 检查选中的卡片是否在可视区域内
    if (selectedCardLeft < containerScrollLeft) {
      // 选中卡片在左侧外部
      const targetLeft = Math.max(0, selectedCardLeft - 20)
      scrollToPosition(targetLeft)
    } else if (selectedCardRight > containerScrollRight) {
      // 选中卡片在右侧外部
      let maxScrollLeft: number
      
      if (scrollbarInstance) {
        maxScrollLeft = scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
      } else {
        const container = clipboardScrollRef.value!
        maxScrollLeft = container.scrollWidth - container.clientWidth
      }
      
      const targetLeft = Math.min(maxScrollLeft, selectedCardRight - containerWidth + 20)
      scrollToPosition(targetLeft)
    }
  })
  
  const scrollToPosition = (targetLeft: number) => {
    if (scrollbarInstance) {
      scrollbarInstance.scrollTo(targetLeft, 0, 300)
    } else if (clipboardScrollRef.value) {
      clipboardScrollRef.value.scrollTo({
        left: targetLeft,
        behavior: 'smooth'
      })
    }
  }
  
  const handleWheelThrottled = useThrottle((event: WheelEvent) => {
    console.log('[滚动标签管理] 收到滚轮事件:', {
      animationState: currentAnimationState.value,
      hasScrollRef: !!clipboardScrollRef.value,
      historyLength: clipboardHistory.value.length,
      deltaX: event.deltaX,
      deltaY: event.deltaY
    })

    // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
    if (currentAnimationState.value !== AnimationState.ACTIVE ||
        !clipboardScrollRef.value ||
        clipboardHistory.value.length === 0) {
      console.log('[滚动标签管理] 滚轮事件被忽略，条件不满足')
      return
    }
    
    // 如果 smooth-scrollbar 未初始化，先初始化
    if (!scrollbarInstance) {
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
      const currentScrollLeft = scrollbarInstance.scrollLeft
      const maxScrollLeft = scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
      const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))
      
      if (targetScrollLeft !== currentScrollLeft) {
        scrollbarInstance.scrollTo(targetScrollLeft, 0, 0)
      }
    } else if (clipboardScrollRef.value) {
      const container = clipboardScrollRef.value
      const currentScrollLeft = container.scrollLeft
      const maxScrollLeft = container.scrollWidth - container.clientWidth
      const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))
      
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
    handleWheelThrottled(event)
  }
  
  const ensureSelectedItemVisible = () => {
    nextTick(() => {
      scrollToSelectedItem()
    })
  }
  
  // ==================== 标签管理 ====================
  
  const loadAllTags = async () => {
    try {
      const tagList = await tags.getAllTags()
      tagStateManager.setState(prev => ({
        ...prev,
        allTags: tagList
      }))
      console.log('[滚动标签管理] 标签数据加载完成:', tagList)
      
      // 加载标签统计数据
      await loadTagStats()
    } catch (error) {
      console.error('[滚动标签管理] 加载标签数据失败:', error)
      tagStateManager.setState(prev => ({
        ...prev,
        allTags: []
      }))
    }
  }
  
  const loadTagStats = async () => {
    try {
      // 模拟统计数据，实际应该从后端获取
      const stats: Record<string, number> = {}
      
      for (const tag of allTags.value) {
        stats[tag.id] = Math.floor(Math.random() * 20) + 1
      }
      
      tagStateManager.setState(prev => ({
        ...prev,
        tagStats: stats
      }))
    } catch (error) {
      console.error('[滚动标签管理] 加载标签统计数据失败:', error)
      tagStateManager.setState(prev => ({
        ...prev,
        tagStats: {}
      }))
    }
  }
  
  const switchTag = async (tagId: string) => {
    if (activeTagId.value === tagId || isTagSwitching.value) return
    
    tagStateManager.setState(prev => ({
      ...prev,
      isTagSwitching: true,
      activeTagId: tagId
    }))
    
    console.log('[滚动标签管理] 切换到标签:', tagId)
    
    try {
      if (tagId === 'all') {
        await loadClipboardHistory()
      } else {
        await loadClipboardHistory({
          tagIds: [tagId],
          limit: 50,
          orderBy: 'last_used',
          orderDirection: 'DESC'
        })
      }
    } catch (error) {
      console.error('[滚动标签管理] 切换标签失败:', error)
    } finally {
      tagStateManager.setState(prev => ({
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
