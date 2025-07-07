// 防抖配置更新的定时器
const debounceTimers = new Map<string, NodeJS.Timeout>()

/**
 * 防抖执行函数
 * @param key 防抖键名
 * @param fn 要执行的函数
 * @param delay 延迟时间（毫秒）
 */
export function debounceExecution(key: string, fn: () => void, delay: number = 500): void {
  // 清除之前的定时器
  const existingTimer = debounceTimers.get(key)
  if (existingTimer) {
    clearTimeout(existingTimer)
  }

  // 设置新的定时器
  const timer = setTimeout(() => {
    fn()
    debounceTimers.delete(key)
  }, delay)

  debounceTimers.set(key, timer)
}

/**
 * 清理所有防抖定时器
 */
export function clearAllDebounceTimers(): void {
  debounceTimers.forEach((timer) => {
    clearTimeout(timer)
  })
  debounceTimers.clear()
}