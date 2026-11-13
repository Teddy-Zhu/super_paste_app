/**
 * 滚动和标签管理 Composable
 * 合并滚动管理和标签管理，提供统一的数据和交互接口
 */

import { ref, computed, nextTick, onUnmounted, readonly, watch, type Ref } from 'vue'
import type { ClipboardItem, TagGroup, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import { tags } from '@renderer/api/renderer-api'
import { useRafThrottle, useStateManager } from './useUtils'
import { AnimationState } from './useAnimation'
import { useContextMenu } from './useContextMenu'
import { getWheelScrollAmount } from '@renderer/utils/wheel'
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
  readonly isDragging: Ref<boolean>

  // 标签状态
  readonly allTags: Ref<TagGroup[]>
  readonly activeTagId: Ref<string>
  readonly isTagSwitching: Ref<boolean>
  readonly tagStats: Ref<Record<string, number>>
  readonly tabs: Ref<TabData[]>

  // 滚动方法
  isAtStart: () => boolean
  scrollToStart: () => void
  scrollToSelectedItem: () => void
  handleWheel: (event: WheelEvent) => void
  handlePointerDown: (event: PointerEvent) => void
  handleClickCapture: (event: MouseEvent) => void
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
const DRAG_THRESHOLD = 6
// 速度单位为 px/ms，加速度单位为 px/ms²。
const MAX_DRAG_VELOCITY = 4
const DRAG_DECELERATION = 0.003
const DRAG_VELOCITY_SMOOTHING = 35
const DRAG_RELEASE_IDLE_TIME = 100
const WHEEL_ACCELERATION_WINDOW = 160

const SCROLLBAR_OPTIONS: Partial<ScrollbarOptions> = {
  damping: 0.12,
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
  isVerticalLayout: Ref<boolean>,
  onViewportChange: (offset: number, size: number, stride: number) => void
): ScrollAndTagsManager {
  // ==================== 状态管理 ====================

  const isScrolling = ref(false)
  const isDragging = ref(false)
  let scrollbarInstance: Scrollbar | null = null
  let lastWheelDirection = 0
  let lastWheelAxis: 'x' | 'y' | undefined
  let lastWheelTime = 0

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
      notifyViewport()
      return
    }

    lastWheelDirection = 0
    lastWheelAxis = undefined
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
      scrollbarInstance.addListener(notifyViewport)
      notifyViewport()
    } catch (error) {
      console.error('[滚动标签管理] smooth-scrollbar 初始化失败:', error)
      scrollbarInstance = null
    }
  }

  const destroyScrollbar = (): void => {
    cancelDragScroll()
    lastWheelDirection = 0
    lastWheelAxis = undefined
    if (scrollbarInstance) {
      try {
        scrollbarInstance.destroy()
        scrollbarInstance = null
      } catch (error) {
        console.error('[滚动标签管理] 销毁 smooth-scrollbar 实例失败:', error)
      }
    }
  }

  const scrollToStart = (): void => {
    cancelDragScroll()
    scrollToSelectedItem.cancel()
    lastWheelDirection = 0
    if (scrollbarInstance) {
      scrollbarInstance.setMomentum(0, 0)
      scrollbarInstance.scrollTo(0, 0, 0)
    } else {
      clipboardScrollRef.value?.scrollTo({ left: 0, top: 0, behavior: 'instant' })
    }
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

  // 卡片在两个布局方向都使用固定主轴尺寸，离屏卡片由等尺寸占位替代。
  const cardStride = (): number => Math.min(300, window.innerWidth - 24) + 10
  const notifyViewport = (): void => {
    const metrics = getScrollMetrics()
    if (!metrics) return
    onViewportChange(
      isVerticalLayout.value ? metrics.scrollTop : metrics.scrollLeft,
      (isVerticalLayout.value
        ? clipboardScrollRef.value?.clientHeight
        : clipboardScrollRef.value?.clientWidth) ||
        (isVerticalLayout.value ? metrics.containerHeight : metrics.containerWidth),
      cardStride()
    )
  }
  const resizeObserver = new ResizeObserver(notifyViewport)
  watch(
    clipboardScrollRef,
    (element, previous) => {
      previous?.removeEventListener('scroll', notifyViewport)
      if (previous) resizeObserver.unobserve(previous)
      element?.addEventListener('scroll', notifyViewport, { passive: true })
      if (element) resizeObserver.observe(element)
    },
    { flush: 'post' }
  )
  window.addEventListener('resize', notifyViewport)

  const isAtStart = (): boolean => {
    const metrics = getScrollMetrics()
    return !metrics || (isVerticalLayout.value ? metrics.scrollTop : metrics.scrollLeft) <= 1
  }

  let drag: {
    pointerId: number
    container: HTMLElement
    startX: number
    startY: number
    lastPosition: number
    lastMoveTime: number
    velocity: number
    vertical: boolean
  } | null = null
  let suppressClick = false
  let dragInertia: { container: HTMLElement; scrollbar: Scrollbar | null } | null = null

  const stopDragInertia = (): void => {
    const previousInertia = dragInertia
    dragInertia = null
    window.removeEventListener('blur', stopDragInertia)
    if (!previousInertia) return
    if (previousInertia.scrollbar) {
      const scrollbar = previousInertia.scrollbar
      scrollbar.scrollTo(scrollbar.scrollLeft, scrollbar.scrollTop, 0)
    } else {
      const container = previousInertia.container
      container.scrollTo({
        left: container.scrollLeft,
        top: container.scrollTop,
        behavior: 'instant'
      })
    }
  }

  const startDragInertia = (velocity: number, vertical: boolean): void => {
    const container = clipboardScrollRef.value
    const metrics = getScrollMetrics()
    if (!container || !metrics || Math.abs(velocity) < 0.08) return
    const start = vertical ? metrics.scrollTop : metrics.scrollLeft
    const limit = Math.max(
      0,
      vertical
        ? metrics.contentHeight - metrics.containerHeight
        : metrics.contentWidth - metrics.containerWidth
    )
    const distance = (velocity * Math.abs(velocity)) / (2 * DRAG_DECELERATION)
    const target = Math.min(limit, Math.max(0, start + distance))
    if (target === start) return
    // 二次缓出对应匀减速；靠近边界时缩短时长，保持松手瞬间的速度。
    const duration = (2 * Math.abs(target - start)) / Math.abs(velocity)
    const left = vertical ? metrics.scrollLeft : target
    const top = vertical ? target : metrics.scrollTop
    dragInertia = { container, scrollbar: scrollbarInstance }
    window.addEventListener('blur', stopDragInertia)
    if (scrollbarInstance) {
      scrollbarInstance.scrollTo(left, top, duration, {
        easing: (progress) => progress * (2 - progress),
        callback: () => {
          dragInertia = null
          window.removeEventListener('blur', stopDragInertia)
        }
      })
    } else {
      container.scrollTo({ left, top, behavior: 'smooth' })
    }
  }

  const cancelDragScroll = (): void => {
    stopDragInertia()
    const previousDrag = drag
    drag = null
    isDragging.value = false
    window.removeEventListener('pointermove', handleDragMove, true)
    window.removeEventListener('pointerup', handleDragEnd, true)
    window.removeEventListener('pointercancel', handleDragEnd, true)
    window.removeEventListener('lostpointercapture', handleDragEnd, true)
    window.removeEventListener('blur', cancelDragScroll)
    if (previousDrag?.container.hasPointerCapture(previousDrag.pointerId)) {
      previousDrag.container.releasePointerCapture(previousDrag.pointerId)
    }
  }

  const handleDragMove = (event: PointerEvent): void => {
    if (!drag || event.pointerId !== drag.pointerId) return
    if (!(event.buttons & 1) || !drag.container.isConnected) {
      cancelDragScroll()
      return
    }
    if (!isDragging.value) {
      // 用位移区分点击和拖拽，容忍点击时的轻微抖动。
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < DRAG_THRESHOLD) {
        return
      }
      scrollToSelectedItem.cancel()
      hideContextMenu()
      if (scrollbarInstance) {
        scrollbarInstance.setMomentum(0, 0)
        scrollbarInstance.scrollTo(scrollbarInstance.scrollLeft, scrollbarInstance.scrollTop, 0)
      }
      lastWheelDirection = 0
      lastWheelAxis = undefined
      isDragging.value = true
      suppressClick = true
      drag.container.setPointerCapture(drag.pointerId)
    }

    event.preventDefault()
    event.stopPropagation()
    const position = drag.vertical ? event.clientY : event.clientX
    const delta = drag.lastPosition - position
    drag.lastPosition = position
    const metrics = getScrollMetrics()
    if (!metrics) return
    const left = metrics.scrollLeft + (drag.vertical ? 0 : delta)
    const top = metrics.scrollTop + (drag.vertical ? delta : 0)
    if (scrollbarInstance) {
      scrollbarInstance.setPosition(left, top)
    } else {
      drag.container.scrollTo({ left, top, behavior: 'instant' })
    }
    const updatedMetrics = getScrollMetrics()
    if (!updatedMetrics) return
    const elapsed = Math.max(1, event.timeStamp - drag.lastMoveTime)
    const moved = drag.vertical
      ? updatedMetrics.scrollTop - metrics.scrollTop
      : updatedMetrics.scrollLeft - metrics.scrollLeft
    const velocity = Math.max(-MAX_DRAG_VELOCITY, Math.min(MAX_DRAG_VELOCITY, moved / elapsed))
    const weight = 1 - Math.exp(-elapsed / DRAG_VELOCITY_SMOOTHING)
    // 到达边界或反向拖拽时丢弃旧速度，避免松手后沿旧方向续滚。
    drag.velocity =
      moved === 0 || Math.sign(velocity) !== Math.sign(drag.velocity)
        ? velocity
        : drag.velocity + (velocity - drag.velocity) * weight
    drag.lastMoveTime = event.timeStamp
  }

  const handleDragEnd = (event: PointerEvent): void => {
    if (!drag || event.pointerId !== drag.pointerId) return
    const { velocity, vertical, lastMoveTime } = drag
    const released = event.type === 'pointerup' && isDragging.value
    const idleFactor = Math.max(0, 1 - (event.timeStamp - lastMoveTime) / DRAG_RELEASE_IDLE_TIME)
    cancelDragScroll()
    if (released) startDragInertia(velocity * idleFactor, vertical)
  }

  const handlePointerDown = (event: PointerEvent): void => {
    if (event.pointerType !== 'mouse' || event.button !== 0) return
    cancelDragScroll()
    suppressClick = false
    const container = clipboardScrollRef.value
    if (
      !container ||
      currentAnimationState.value !== AnimationState.ACTIVE ||
      !(event.target instanceof Element) ||
      event.target.closest(
        'button, a, input, textarea, select, [contenteditable], .scrollbar-track'
      )
    )
      return

    const metrics = getScrollMetrics()
    const vertical = isVerticalLayout.value
    if (
      !metrics ||
      (vertical
        ? metrics.contentHeight <= metrics.containerHeight
        : metrics.contentWidth <= metrics.containerWidth)
    )
      return

    // 阻止原生文字选择和图片拖动；未拖拽时仍由原有 click 处理。
    event.preventDefault()
    scrollToSelectedItem.cancel()
    if (scrollbarInstance) {
      scrollbarInstance.setMomentum(0, 0)
      scrollbarInstance.scrollTo(scrollbarInstance.scrollLeft, scrollbarInstance.scrollTop, 0)
    }
    drag = {
      pointerId: event.pointerId,
      container,
      startX: event.clientX,
      startY: event.clientY,
      lastPosition: vertical ? event.clientY : event.clientX,
      lastMoveTime: event.timeStamp,
      velocity: 0,
      vertical
    }
    window.addEventListener('pointermove', handleDragMove, true)
    window.addEventListener('pointerup', handleDragEnd, true)
    window.addEventListener('pointercancel', handleDragEnd, true)
    window.addEventListener('lostpointercapture', handleDragEnd, true)
    window.addEventListener('blur', cancelDragScroll)
  }

  const handleClickCapture = (event: MouseEvent): void => {
    // 保留键盘触发的点击；拖拽后的鼠标点击不得触发卡片粘贴。
    if (!suppressClick || event.detail === 0) return
    suppressClick = false
    event.preventDefault()
    event.stopPropagation()
  }

  watch([clipboardScrollRef, isVerticalLayout, currentAnimationState], cancelDragScroll)

  const scrollToAxisPosition = (target: number, axis: 'x' | 'y'): void => {
    stopDragInertia()
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
    const metrics = getScrollMetrics()
    if (!metrics) {
      return
    }

    const verticalLayout = isVerticalLayout.value
    const containerStart = verticalLayout ? metrics.scrollTop : metrics.scrollLeft
    const containerSize = verticalLayout ? metrics.containerHeight : metrics.containerWidth
    const contentSize = verticalLayout ? metrics.contentHeight : metrics.contentWidth
    const maxScroll = Math.max(0, contentSize - containerSize)

    const selectedStart = selectedIndex.value * cardStride()
    const selectedSize = cardStride() - 10
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

    const metrics = getScrollMetrics()
    if (!metrics) {
      return false
    }

    const verticalLayout = isVerticalLayout.value
    const containerStart = verticalLayout ? metrics.scrollTop : metrics.scrollLeft
    const containerSize = verticalLayout ? metrics.containerHeight : metrics.containerWidth
    const containerEnd = containerStart + containerSize

    const selectedStart = selectedIndex.value * cardStride()
    const selectedSize = cardStride() - 10
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

    cancelDragScroll()

    const verticalLayout = isVerticalLayout.value
    const scrollAmount = getWheelScrollAmount(
      event,
      verticalLayout ? container.clientHeight : container.clientWidth
    )
    if (scrollAmount === 0) return

    const direction = Math.sign(scrollAmount)
    const axis = verticalLayout ? 'y' : 'x'
    const now = performance.now()
    const sameDirection = direction === lastWheelDirection && axis === lastWheelAxis
    const acceleration = sameDirection
      ? Math.max(0, 1 - (now - lastWheelTime) / WHEEL_ACCELERATION_WINDOW)
      : 0
    const momentum = scrollAmount * (1.5 + acceleration)
    lastWheelTime = now

    scrollToSelectedItem.cancel()
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
        // 取消键盘定位动画，但保留同向滚动的剩余位移，让连续滚轮完整累积。
        scrollbarInstance.scrollTo(scrollbarInstance.scrollLeft, scrollbarInstance.scrollTop, 0)
        if (!sameDirection) {
          scrollbarInstance.setMomentum(0, 0)
        }
        lastWheelDirection = direction
        lastWheelAxis = axis
        scrollbarInstance.addMomentum(verticalLayout ? 0 : momentum, verticalLayout ? momentum : 0)
      } catch (error) {
        console.warn('[滚动标签管理] scrollbar 实例操作失败，尝试重新初始化:', error)
        initializeScrollbar()
      }
    } else {
      container.scrollBy({
        left: verticalLayout ? 0 : momentum,
        top: verticalLayout ? momentum : 0,
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
        limit: 24,
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
    resizeObserver.disconnect()
    clipboardScrollRef.value?.removeEventListener('scroll', notifyViewport)
    window.removeEventListener('resize', notifyViewport)
    destroyScrollbar()
  })

  // ==================== 返回接口 ====================

  return {
    // 滚动状态
    isScrolling: readonly(isScrolling),
    isDragging: readonly(isDragging),

    // 标签状态
    allTags,
    activeTagId: readonly(activeTagId),
    isTagSwitching: readonly(isTagSwitching),
    tagStats: readonly(tagStats),
    tabs,

    // 滚动方法
    isAtStart,
    scrollToStart,
    scrollToSelectedItem,
    handleWheel,
    handlePointerDown,
    handleClickCapture,
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
