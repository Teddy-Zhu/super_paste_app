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

/**
 * 哈希工具类
 * 统一管理所有哈希生成逻辑，避免重复代码
 */
export class HashUtils {
  /**
   * 生成字符串哈希值
   * 使用简单的哈希算法，适用于快速比较
   */
  static generateStringHash(content: string): string {
    let hash = 0
    for (let i = 0; i < content.length; i++) {
      const char = content.charCodeAt(i)
      hash = ((hash << 5) - hash) + char
      hash = hash & hash
    }
    return hash.toString()
  }

  /**
   * 生成对象内容哈希值
   * 将对象序列化后生成哈希
   */
  static generateContentHash(content: any): string {
    const contentStr = JSON.stringify(content)
    return this.generateStringHash(contentStr)
  }

  /**
   * 生成快速哈希（用于剪贴板变化检测）
   * 不包含图片具体内容，只检查基本格式和图片基本信息
   */
  static generateQuickHash(content: {
    text?: string
    html?: string
    rtf?: string
    hasImage?: boolean
    hasFiles?: boolean
    imageInfo?: { width: number; height: number; quickId: string }
    filesInfo?: { count: number; totalSize: number }
  }): string {
    const quickContent = {
      text: content.text || undefined,
      html: content.html || undefined,
      rtf: content.rtf || undefined,
      hasImage: content.hasImage,
      hasFiles: content.hasFiles,
      imageInfo: content.imageInfo || undefined,
      filesInfo: content.filesInfo || undefined
    }
    return this.generateContentHash(quickContent)
  }

  /**
   * 生成图片内容的优化哈希
   * 对大图片使用采样策略减少CPU压力
   */
  static generateImageHash(imageData: string): string {
    if (!imageData) return ''

    // 对于大图片，只使用前1000和后1000字符生成哈希
    if (imageData.length > 10000) {
      const start = imageData.substring(0, 1000)
      const end = imageData.substring(imageData.length - 1000)
      const sampleData = start + end + imageData.length.toString()
      return this.generateStringHash(sampleData)
    }

    return this.generateStringHash(imageData)
  }
}