/**
 * 统一工具函数库
 * 提供公共的工具函数、事件管理器和状态管理器
 */

import { ref, onUnmounted, nextTick, readonly, type Ref } from 'vue'

// ==================== 类型定义 ====================

export interface EventListenerCleanup {
  (): void
}

export interface StateManager<T> {
  state: Readonly<Ref<T>>
  previousState: Readonly<Ref<T>>
  setState: (newState: T | ((prev: T) => T)) => void
  resetState: () => void
  revertState: () => void
}

export interface EventManager {
  addEventListener: (
    element: EventTarget,
    event: string,
    handler: EventListener,
    options?: AddEventListenerOptions
  ) => EventListenerCleanup
  removeAllListeners: () => void
  cleanup: () => void
}

export interface TimerManager {
  setTimeout: (callback: () => void, delay: number) => number
  setInterval: (callback: () => void, delay: number) => number
  requestAnimationFrame: (callback: FrameRequestCallback) => number
  clearTimeout: (id: number) => void
  clearInterval: (id: number) => void
  cancelAnimationFrame: (id: number) => void
  cleanup: () => void
}

export interface DebounceFunction<T extends (...args: any[]) => any> {
  (...args: Parameters<T>): void
  cancel: () => void
}

export interface ThrottleFunction<T extends (...args: any[]) => any> {
  (...args: Parameters<T>): void
  cancel: () => void
}

// ==================== 事件管理器 ====================

/**
 * 创建事件管理器
 * 统一管理事件监听器，自动清理
 */
export function useEventManager(): EventManager {
  const listeners = ref<Array<{
    element: EventTarget
    event: string
    handler: EventListener
    cleanup: () => void
  }>>([])

  const addEventListener = (
    element: EventTarget,
    event: string,
    handler: EventListener,
    options?: AddEventListenerOptions
  ): EventListenerCleanup => {
    element.addEventListener(event, handler, options)
    
    const cleanup = () => {
      element.removeEventListener(event, handler)
      const index = listeners.value.findIndex(l => 
        l.element === element && l.event === event && l.handler === handler
      )
      if (index > -1) {
        listeners.value.splice(index, 1)
      }
    }

    listeners.value.push({ element, event, handler, cleanup })
    return cleanup
  }

  const removeAllListeners = () => {
    listeners.value.forEach(({ cleanup }) => cleanup())
    listeners.value = []
  }

  const cleanup = removeAllListeners

  // 组件卸载时自动清理
  onUnmounted(cleanup)

  return {
    addEventListener,
    removeAllListeners,
    cleanup
  }
}

// ==================== 定时器管理器 ====================

/**
 * 创建定时器管理器
 * 统一管理定时器，防止内存泄漏
 */
export function useTimerManager(): TimerManager {
  const timeouts = ref<Set<number>>(new Set())
  const intervals = ref<Set<number>>(new Set())
  const rafs = ref<Set<number>>(new Set())

  const setTimeout = (callback: () => void, delay: number): number => {
    const id = window.setTimeout(() => {
      timeouts.value.delete(id)
      callback()
    }, delay)
    timeouts.value.add(id)
    return id
  }

  const setInterval = (callback: () => void, delay: number): number => {
    const id = window.setInterval(callback, delay)
    intervals.value.add(id)
    return id
  }

  const requestAnimationFrame = (callback: FrameRequestCallback): number => {
    const id = window.requestAnimationFrame((time) => {
      rafs.value.delete(id)
      callback(time)
    })
    rafs.value.add(id)
    return id
  }

  const clearTimeout = (id: number) => {
    window.clearTimeout(id)
    timeouts.value.delete(id)
  }

  const clearInterval = (id: number) => {
    window.clearInterval(id)
    intervals.value.delete(id)
  }

  const cancelAnimationFrame = (id: number) => {
    window.cancelAnimationFrame(id)
    rafs.value.delete(id)
  }

  const cleanup = () => {
    timeouts.value.forEach(id => window.clearTimeout(id))
    intervals.value.forEach(id => window.clearInterval(id))
    rafs.value.forEach(id => window.cancelAnimationFrame(id))
    
    timeouts.value.clear()
    intervals.value.clear()
    rafs.value.clear()
  }

  // 组件卸载时自动清理
  onUnmounted(cleanup)

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

// ==================== 防抖函数 ====================

/**
 * 创建防抖函数
 * @param fn 要防抖的函数
 * @param delay 延迟时间（毫秒）
 */
export function useDebounce<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 300
): DebounceFunction<T> {
  const timerManager = useTimerManager()
  let timeoutId: number | null = null

  const debouncedFn = ((...args: Parameters<T>) => {
    if (timeoutId !== null) {
      timerManager.clearTimeout(timeoutId)
    }
    
    timeoutId = timerManager.setTimeout(() => {
      fn(...args)
      timeoutId = null
    }, delay)
  }) as DebounceFunction<T>

  debouncedFn.cancel = () => {
    if (timeoutId !== null) {
      timerManager.clearTimeout(timeoutId)
      timeoutId = null
    }
  }

  return debouncedFn
}

// ==================== 节流函数 ====================

/**
 * 创建节流函数
 * @param fn 要节流的函数
 * @param delay 节流间隔（毫秒）
 */
export function useThrottle<T extends (...args: any[]) => any>(
  fn: T,
  delay: number = 100
): ThrottleFunction<T> {
  const timerManager = useTimerManager()
  let isThrottled = false
  let timeoutId: number | null = null

  const throttledFn = ((...args: Parameters<T>) => {
    if (isThrottled) return

    fn(...args)
    isThrottled = true

    timeoutId = timerManager.setTimeout(() => {
      isThrottled = false
      timeoutId = null
    }, delay)
  }) as ThrottleFunction<T>

  throttledFn.cancel = () => {
    if (timeoutId !== null) {
      timerManager.clearTimeout(timeoutId)
      timeoutId = null
    }
    isThrottled = false
  }

  return throttledFn
}

// ==================== RAF 节流函数 ====================

/**
 * 创建 RAF 节流函数（用于动画和滚动）
 * @param fn 要节流的函数
 */
export function useRafThrottle<T extends (...args: any[]) => any>(
  fn: T
): ThrottleFunction<T> {
  const timerManager = useTimerManager()
  let rafId: number | null = null
  let isScheduled = false

  const rafThrottledFn = ((...args: Parameters<T>) => {
    if (isScheduled) return

    isScheduled = true
    rafId = timerManager.requestAnimationFrame(() => {
      fn(...args)
      isScheduled = false
      rafId = null
    })
  }) as ThrottleFunction<T>

  rafThrottledFn.cancel = () => {
    if (rafId !== null) {
      timerManager.cancelAnimationFrame(rafId)
      rafId = null
    }
    isScheduled = false
  }

  return rafThrottledFn
}

// ==================== 异步工具函数 ====================

/**
 * 创建带超时的 Promise
 * @param promise 原始 Promise
 * @param timeout 超时时间（毫秒）
 * @param errorMessage 超时错误信息
 */
export function withTimeout<T>(
  promise: Promise<T>,
  timeout: number,
  errorMessage: string = '操作超时'
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(errorMessage))
    }, timeout)

    promise
      .then(resolve)
      .catch(reject)
      .finally(() => clearTimeout(timer))
  })
}

/**
 * 等待 DOM 更新完成
 */
export function waitForNextTick(): Promise<void> {
  return new Promise(resolve => {
    nextTick(() => resolve())
  })
}

/**
 * 等待指定时间
 * @param ms 等待时间（毫秒）
 */
export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}

// ==================== 状态管理工具 ====================

/**
 * 创建状态管理器
 * @param initialState 初始状态
 */
export function useStateManager<T>(initialState: T): StateManager<T> {
  const state = ref<T>(initialState)
  const previousState = ref<T>(initialState)

  const setState = (newState: T | ((prev: T) => T)) => {
    previousState.value = state.value
    if (typeof newState === 'function') {
      state.value = (newState as (prev: T) => T)(state.value)
    } else {
      state.value = newState
    }
  }

  const resetState = () => {
    setState(initialState)
  }

  const revertState = () => {
    setState(previousState.value)
  }

  return {
    state: readonly(state) as Readonly<Ref<T>>,
    previousState: readonly(previousState) as Readonly<Ref<T>>,
    setState,
    resetState,
    revertState
  }
}
