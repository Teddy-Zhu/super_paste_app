/**
 * 性能优化 Composable
 * 提供防抖、节流、内存管理等性能优化功能
 */

import { ref, onUnmounted } from 'vue'
import type {
  DebouncedFunction,
  ThrottledFunction,
  CancelFunction,
  EventListenerManager,
  MemoryLeakGuard,
  BatchUpdateManager
} from '@renderer/types/overlay'

/**
 * 创建防抖函数
 */
export function useDebounce<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 300
): [DebouncedFunction<T>, CancelFunction] {
  const timeoutRef = ref<NodeJS.Timeout | null>(null)

  const debouncedFn = ((...args: Parameters<T>) => {
    if (timeoutRef.value) {
      clearTimeout(timeoutRef.value)
    }
    
    timeoutRef.value = setTimeout(() => {
      fn(...args)
    }, delay)
  }) as T

  const cancel = () => {
    if (timeoutRef.value) {
      clearTimeout(timeoutRef.value)
      timeoutRef.value = null
    }
  }

  // 组件卸载时自动清理
  onUnmounted(() => {
    cancel()
  })

  return [debouncedFn, cancel]
}

/**
 * 创建节流函数
 */
export function useThrottle<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 100
): [ThrottledFunction<T>, CancelFunction] {
  const isThrottled = ref(false)
  const timeoutRef = ref<NodeJS.Timeout | null>(null)

  const throttledFn = ((...args: Parameters<T>) => {
    if (isThrottled.value) return

    fn(...args)
    isThrottled.value = true

    timeoutRef.value = setTimeout(() => {
      isThrottled.value = false
    }, delay)
  }) as T

  const cancel = () => {
    if (timeoutRef.value) {
      clearTimeout(timeoutRef.value)
      timeoutRef.value = null
    }
    isThrottled.value = false
  }

  // 组件卸载时自动清理
  onUnmounted(() => {
    cancel()
  })

  return [throttledFn, cancel]
}

/**
 * 创建 RAF 节流函数（用于动画和滚动）
 */
export function useRafThrottle<T extends (...args: any[]) => any>(
  fn: T
): [ThrottledFunction<T>, CancelFunction] {
  const rafId = ref<number | null>(null)
  const isScheduled = ref(false)

  const rafThrottledFn = ((...args: Parameters<T>) => {
    if (isScheduled.value) return

    isScheduled.value = true
    rafId.value = requestAnimationFrame(() => {
      fn(...args)
      isScheduled.value = false
    })
  }) as T

  const cancel = () => {
    if (rafId.value) {
      cancelAnimationFrame(rafId.value)
      rafId.value = null
    }
    isScheduled.value = false
  }

  // 组件卸载时自动清理
  onUnmounted(() => {
    cancel()
  })

  return [rafThrottledFn, cancel]
}

/**
 * 事件监听器管理器
 */
export function useEventListenerManager(): EventListenerManager {
  const listeners = ref<Array<{
    element: EventTarget
    event: string
    handler: EventListener
    options?: AddEventListenerOptions
  }>>([])

  const addEventListener = (
    element: EventTarget,
    event: string,
    handler: EventListener,
    options?: AddEventListenerOptions
  ) => {
    element.addEventListener(event, handler, options)
    listeners.value.push({ element, event, handler, options })
  }

  const removeEventListener = (
    element: EventTarget,
    event: string,
    handler: EventListener
  ) => {
    element.removeEventListener(event, handler)
    const index = listeners.value.findIndex(
      l => l.element === element && l.event === event && l.handler === handler
    )
    if (index > -1) {
      listeners.value.splice(index, 1)
    }
  }

  const removeAllListeners = () => {
    listeners.value.forEach(({ element, event, handler }) => {
      element.removeEventListener(event, handler)
    })
    listeners.value = []
  }

  // 组件卸载时自动清理所有监听器
  onUnmounted(() => {
    removeAllListeners()
  })

  return {
    addEventListener,
    removeEventListener,
    removeAllListeners
  }
}

/**
 * 内存泄漏防护
 */
export function useMemoryLeakGuard(): MemoryLeakGuard {
  const timers = ref<Set<NodeJS.Timeout>>(new Set())
  const intervals = ref<Set<NodeJS.Timeout>>(new Set())
  const rafs = ref<Set<number>>(new Set())

  const setTimeout = (callback: () => void, delay: number) => {
    const timer = globalThis.setTimeout(() => {
      timers.value.delete(timer)
      callback()
    }, delay)
    timers.value.add(timer)
    return timer
  }

  const setInterval = (callback: () => void, delay: number) => {
    const interval = globalThis.setInterval(callback, delay)
    intervals.value.add(interval)
    return interval
  }

  const requestAnimationFrame = (callback: FrameRequestCallback) => {
    const raf = globalThis.requestAnimationFrame((time) => {
      rafs.value.delete(raf)
      callback(time)
    })
    rafs.value.add(raf)
    return raf
  }

  const clearTimeout = (timer: NodeJS.Timeout) => {
    globalThis.clearTimeout(timer)
    timers.value.delete(timer)
  }

  const clearInterval = (interval: NodeJS.Timeout) => {
    globalThis.clearInterval(interval)
    intervals.value.delete(interval)
  }

  const cancelAnimationFrame = (raf: number) => {
    globalThis.cancelAnimationFrame(raf)
    rafs.value.delete(raf)
  }

  const cleanup = () => {
    timers.value.forEach(timer => globalThis.clearTimeout(timer))
    intervals.value.forEach(interval => globalThis.clearInterval(interval))
    rafs.value.forEach(raf => globalThis.cancelAnimationFrame(raf))
    
    timers.value.clear()
    intervals.value.clear()
    rafs.value.clear()
  }

  // 组件卸载时自动清理
  onUnmounted(() => {
    cleanup()
  })

  return {
    setTimeout,
    setInterval,
    requestAnimationFrame,
    clearTimeout,
    clearInterval,
    cancelAnimationFrame,
    cleanup
  }
}

/**
 * 批量更新优化
 */
export function useBatchUpdate<T>(
  updateFn: (items: T[]) => void,
  delay: number = 16 // 一帧的时间
): BatchUpdateManager<T> {
  const pendingUpdates = ref<T[]>([])
  const timeoutRef = ref<NodeJS.Timeout | null>(null)

  const addUpdate = (item: T) => {
    (pendingUpdates.value as T[]).push(item)

    if (!timeoutRef.value) {
      timeoutRef.value = setTimeout(() => {
        if (pendingUpdates.value.length > 0) {
          updateFn([...(pendingUpdates.value as T[])])
          pendingUpdates.value = []
        }
        timeoutRef.value = null
      }, delay)
    }
  }

  const flush = () => {
    if (timeoutRef.value) {
      clearTimeout(timeoutRef.value)
      timeoutRef.value = null
    }

    if (pendingUpdates.value.length > 0) {
      updateFn([...(pendingUpdates.value as T[])])
      pendingUpdates.value = []
    }
  }

  // 组件卸载时自动清理
  onUnmounted(() => {
    if (timeoutRef.value) {
      clearTimeout(timeoutRef.value)
    }
  })

  return {
    addUpdate,
    flush
  }
}
