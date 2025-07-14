/**
 * 性能优化 Composable
 * 提供批量更新、虚拟滚动等高级性能优化功能
 * 基础的防抖、节流功能已移至 useUtils.ts
 */

import { ref, readonly, type Ref } from 'vue'
import { useTimerManager } from './useUtils'

// ==================== 类型定义 ====================

export interface BatchUpdateManager<T> {
  addUpdate: (item: T) => void
  flush: () => void
  clear: () => void
}

export interface VirtualScrollManager<T> {
  visibleItems: Ref<T[]>
  startIndex: Ref<number>
  endIndex: Ref<number>
  totalHeight: Ref<number>
  updateVisibleRange: (scrollTop: number, containerHeight: number) => void
  getItemHeight: (index: number) => number
  setItems: (items: T[]) => void
}

export interface PerformanceMonitor {
  startTiming: (label: string) => void
  endTiming: (label: string) => void
  getTimings: () => Record<string, number>
  clearTimings: () => void
}

// ==================== 批量更新管理器 ====================

/**
 * 创建批量更新管理器
 * 将多个更新操作合并为一次执行，减少重绘
 */
export function useBatchUpdate<T>(
  updateFn: (items: T[]) => void,
  delay: number = 16 // 一帧的时间
): BatchUpdateManager<T> {
  const timerManager = useTimerManager()
  const pendingUpdates = ref<T[]>([])
  let timeoutId: number | null = null

  const addUpdate = (item: T) => {
    pendingUpdates.value.push(item as any)

    if (timeoutId === null) {
      timeoutId = timerManager.setTimeout(() => {
        if (pendingUpdates.value.length > 0) {
          updateFn([...pendingUpdates.value] as T[])
          pendingUpdates.value = []
        }
        timeoutId = null
      }, delay)
    }
  }

  const flush = () => {
    if (timeoutId !== null) {
      timerManager.clearTimeout(timeoutId)
      timeoutId = null
    }

    if (pendingUpdates.value.length > 0) {
      updateFn([...pendingUpdates.value] as T[])
      pendingUpdates.value = []
    }
  }

  const clear = () => {
    if (timeoutId !== null) {
      timerManager.clearTimeout(timeoutId)
      timeoutId = null
    }
    pendingUpdates.value = []
  }

  return {
    addUpdate,
    flush,
    clear
  }
}

// ==================== 虚拟滚动管理器 ====================

/**
 * 创建虚拟滚动管理器
 * 用于大量数据的高性能渲染
 */
export function useVirtualScroll<T>(
  itemHeight: number = 50,
  overscan: number = 5
): VirtualScrollManager<T> {
  const items = ref<T[]>([])
  const visibleItems = ref<T[]>([])
  const startIndex = ref(0)
  const endIndex = ref(0)
  const totalHeight = ref(0)

  const updateVisibleRange = (scrollTop: number, containerHeight: number) => {
    const start = Math.floor(scrollTop / itemHeight)
    const visibleCount = Math.ceil(containerHeight / itemHeight)
    const end = start + visibleCount

    // 添加 overscan 以提供更好的滚动体验
    const newStartIndex = Math.max(0, start - overscan)
    const newEndIndex = Math.min(items.value.length - 1, end + overscan)

    startIndex.value = newStartIndex
    endIndex.value = newEndIndex

    // 更新可见项目
    visibleItems.value = items.value.slice(newStartIndex, newEndIndex + 1)
  }

  const getItemHeight = (_index: number): number => {
    return itemHeight
  }

  const setItems = (newItems: T[]) => {
    items.value = newItems as any
    totalHeight.value = newItems.length * itemHeight

    // 重新计算可见范围
    if (newItems.length > 0) {
      updateVisibleRange(0, 0) // 重置到顶部
    }
  }

  return {
    visibleItems: visibleItems as Ref<T[]>,
    startIndex: readonly(startIndex),
    endIndex: readonly(endIndex),
    totalHeight: readonly(totalHeight),
    updateVisibleRange,
    getItemHeight,
    setItems
  }
}

// ==================== 性能监控器 ====================

/**
 * 创建性能监控器
 * 用于监控关键操作的执行时间
 */
export function usePerformanceMonitor(): PerformanceMonitor {
  const timings = ref<Record<string, number>>({})
  const startTimes = ref<Record<string, number>>({})

  const startTiming = (label: string) => {
    startTimes.value[label] = performance.now()
  }

  const endTiming = (label: string) => {
    const startTime = startTimes.value[label]
    if (startTime !== undefined) {
      const duration = performance.now() - startTime
      timings.value[label] = duration
      delete startTimes.value[label]
      console.log(`[性能监控] ${label}: ${duration.toFixed(2)}ms`)
    }
  }

  const getTimings = (): Record<string, number> => {
    return { ...timings.value }
  }

  const clearTimings = () => {
    timings.value = {}
    startTimes.value = {}
  }

  return {
    startTiming,
    endTiming,
    getTimings,
    clearTimings
  }
}

// ==================== 导出兼容性函数 ====================

// 为了保持向后兼容，重新导出 useUtils 中的函数
export { useDebounce, useThrottle, useRafThrottle } from './useUtils'
