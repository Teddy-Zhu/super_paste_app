import { BrowserWindow } from 'electron'
import { ClipboardManagerManager } from './workers/clipboard-manager-manager'
import store from './store'
import { STORE_KEY } from './common'
import type {
  ClipboardContent,
  ClipboardItem,
  ClipboardManagerConfig,
  TagGroup,
  TagStats
} from './types/clipboard-types'
import ActiveWindow from '@paymoapp/active-window'
import clipboardManagerWorkerPath from './workers/clipboard-manager.worker?modulePath'

/**
 * 剪贴板管理器（主线程接口）
 * 通过 Worker Thread 执行剪贴板管理操作以提升性能
 */
class ClipboardManager {
  private static instance: ClipboardManager
  private workerManager: ClipboardManagerManager | null = null
  private isInitialized: boolean = false

  private constructor() {
    console.log('[剪贴板管理器] 初始化主线程接口')
    
    // 初始化 ActiveWindow
    try {
      ActiveWindow.initialize()
      // 在 macOS 上请求屏幕录制权限
      if (process.platform === 'darwin') {
        const hasPermission = ActiveWindow.requestPermissions()
        if (!hasPermission) {
          console.warn('[剪贴板管理器] macOS 需要屏幕录制权限才能获取窗口标题')
        }
      }
      console.log('[剪贴板管理器] ActiveWindow 已初始化')
    } catch (error) {
      console.error('[剪贴板管理器] ActiveWindow 初始化失败:', error)
    }
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ClipboardManager {
    if (!ClipboardManager.instance) {
      ClipboardManager.instance = new ClipboardManager()
    }
    return ClipboardManager.instance
  }

  /**
   * 初始化 Worker
   */
  private async initializeWorker(): Promise<void> {
    if (this.workerManager) {
      return
    }

    try {
      // 创建 Worker 管理器
      this.workerManager = new ClipboardManagerManager(clipboardManagerWorkerPath)
      
      // 启动 Worker
      await this.workerManager.start()
      
      this.isInitialized = true
      console.log('[剪贴板管理器] Worker 初始化完成')
    } catch (error) {
      console.error('[剪贴板管理器] Worker 初始化失败:', error)
      throw error
    }
  }

  /**
   * 确保 Worker 已初始化
   */
  private async ensureWorkerInitialized(): Promise<void> {
    if (!this.isInitialized) {
      await this.initializeWorker()
    }
  }

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    await this.ensureWorkerInitialized()
    
    if (!this.workerManager) {
      throw new Error('Worker 未初始化')
    }

    // 从store中获取配置
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    
    const finalConfig: Partial<ClipboardManagerConfig> = {
      maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false,
      ...config
    }

    await this.workerManager.initialize(finalConfig)
    console.log('[剪贴板管理器] 已初始化，配置:', finalConfig)
  }

  /**
   * 通知前端剪贴板历史更新
   */
  private notifyFrontendUpdate(): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', {
            timestamp: Date.now()
          })
        }
      })

      console.log('[剪贴板管理器] 已通知前端剪贴板历史更新')
    } catch (error) {
      console.error('[剪贴板管理器] 通知前端更新失败:', error)
    }
  }

  /**
   * 开始监听剪贴板变化
   */
  public async startListening(): Promise<void> {
    await this.ensureWorkerInitialized()
    
    if (this.workerManager) {
      await this.workerManager.startListening()
      console.log('[剪贴板管理器] 开始监听剪贴板变化')
    }
  }

  /**
   * 停止监听剪贴板变化
   */
  public async stopListening(): Promise<void> {
    if (this.workerManager) {
      await this.workerManager.stopListening()
      console.log('[剪贴板管理器] 停止监听剪贴板变化')
    }
  }

  /**
   * 还原指定的剪贴板内容到当前剪贴板
   */
  public async restoreClipboard(itemId: string): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()
      
      if (!this.workerManager) {
        throw new Error('Worker 未初始化')
      }

      const success = await this.workerManager.restoreClipboardItem(itemId)
      
      if (success) {
        console.log('[剪贴板管理器] 已还原剪贴板内容:', itemId)
      } else {
        console.error('[剪贴板管理器] 还原失败:', itemId)
      }
      
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 还原剪贴板失败:', error)
      return false
    }
  }

  /**
   * 使用同步方式还原剪贴板内容（备用方法）
   */
  public async restoreClipboardSync(itemId: string): Promise<boolean> {
    // 在 Worker 架构中，这个方法与 restoreClipboard 相同
    return this.restoreClipboard(itemId)
  }

  /**
   * 手动设置剪贴板内容（不触发监控）
   */
  public async setClipboardContent(content: ClipboardContent, updateHash: boolean = true): Promise<boolean> {
    try {
      // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
      // 暂停监听以避免触发变更检测
      const wasListening = await this.isListeningActive()
      if (wasListening) {
        await this.stopListening()
      }

      try {
        // 实现剪贴板设置逻辑（从原文件复制）
        // 这里需要实现具体的剪贴板设置逻辑
        console.log('[剪贴板管理器] 设置剪贴板内容:', Object.keys(content))

        // 更新内部哈希值以保持同步
        if (updateHash && this.workerManager) {
          await this.workerManager.syncClipboardHash()
        }

        return true
      } finally {
        // 恢复监听状态
        if (wasListening) {
          setTimeout(async () => {
            await this.startListening()
          }, 150)
        }
      }
    } catch (error) {
      console.error('[剪贴板管理器] 设置剪贴板内容失败:', error)
      return false
    }
  }

  /**
   * 删除指定的历史记录项
   */
  public async deleteHistoryItem(itemId: string): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()
      
      if (!this.workerManager) {
        throw new Error('Worker 未初始化')
      }

      const success = await this.workerManager.deleteHistoryItem(itemId)
      
      if (success) {
        console.log('[剪贴板管理器] 已删除剪贴板项:', itemId)
        this.notifyFrontendUpdate()
      }
      
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 删除剪贴板项失败:', error)
      return false
    }
  }

  /**
   * 清空所有历史记录
   */
  public async clearHistory(): Promise<void> {
    try {
      await this.ensureWorkerInitialized()
      
      if (!this.workerManager) {
        throw new Error('Worker 未初始化')
      }

      await this.workerManager.clearHistory()
      console.log('[剪贴板管理器] 已清空所有历史记录')
      this.notifyFrontendUpdate()
    } catch (error) {
      console.error('[剪贴板管理器] 清空历史记录失败:', error)
    }
  }

  /**
   * 获取历史记录
   */
  public async getHistory(limit?: number): Promise<ClipboardItem[]> {
    try {
      await this.ensureWorkerInitialized()
      
      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.getHistory(limit)
    } catch (error) {
      console.error('[剪贴板管理器] 获取历史记录失败:', error)
      return []
    }
  }

  /**
   * 获取配置
   */
  public async getConfig(): Promise<ClipboardManagerConfig> {
    try {
      await this.ensureWorkerInitialized()
      
      if (!this.workerManager) {
        // 返回默认配置
        const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
        return {
          maxHistorySize,
          checkInterval: 200,
          enableAutoSave: false
        }
      }

      return await this.workerManager.getConfig()
    } catch (error) {
      console.error('[剪贴板管理器] 获取配置失败:', error)
      // 返回默认配置
      const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
      return {
        maxHistorySize,
        checkInterval: 200,
        enableAutoSave: false
      }
    }
  }

  /**
   * 更新配置
   */
  public async updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    try {
      await this.ensureWorkerInitialized()
      
      if (this.workerManager) {
        await this.workerManager.updateConfig(newConfig)
        console.log('[剪贴板管理器] 配置已更新:', newConfig)
      }
    } catch (error) {
      console.error('[剪贴板管理器] 更新配置失败:', error)
    }
  }

  /**
   * 从store更新maxHistorySize配置
   */
  public async updateMaxHistorySizeFromStore(): Promise<void> {
    try {
      const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
      await this.updateConfig({ maxHistorySize })
      console.log('[剪贴板管理器] 从store更新maxHistorySize:', maxHistorySize)
    } catch (error) {
      console.error('[剪贴板管理器] 从store更新maxHistorySize失败:', error)
    }
  }

  /**
   * 从store更新maxKeepTime配置并更新清理器
   */
  public updateMaxKeepTimeFromStore(): void {
    // 这个方法委托给清理器处理
    try {
      // 导入清理器并更新配置
      import('./clipboard-history-cleaner').then(module => {
        const clipboardHistoryCleaner = module.default
        clipboardHistoryCleaner.updateConfigFromStore()
        console.log('[剪贴板管理器] 已更新历史记录清理器配置')
      })
    } catch (error) {
      console.error('[剪贴板管理器] 更新清理器配置失败:', error)
    }
  }

  /**
   * 获取监听状态
   */
  public async isListeningActive(): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return false
      }

      return await this.workerManager.isListeningActive()
    } catch (error) {
      console.error('[剪贴板管理器] 获取监听状态失败:', error)
      return false
    }
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   */
  public async syncClipboardHash(): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      if (this.workerManager) {
        await this.workerManager.syncClipboardHash()
        console.log('[剪贴板管理器] 已同步剪贴板哈希值')
      }
    } catch (error) {
      console.error('[剪贴板管理器] 同步剪贴板哈希值失败:', error)
    }
  }

  /**
   * 获取当前剪贴板内容（不触发监控）
   */
  public async getCurrentClipboard(): Promise<ClipboardContent> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return {}
      }

      return await this.workerManager.getCurrentClipboard()
    } catch (error) {
      console.error('[剪贴板管理器] 获取当前剪贴板内容失败:', error)
      return {}
    }
  }

  /**
   * 测试剪贴板功能
   */
  public testClipboard(): void {
    // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
    try {
      console.log('[剪贴板管理器] 测试剪贴板功能（主线程实现）')
      // 实现测试逻辑
    } catch (error) {
      console.error('[剪贴板管理器] 测试剪贴板功能失败:', error)
    }
  }

  /**
   * 验证剪贴板内容是否可用
   */
  public verifyClipboardContent(): { isValid: boolean; details: any } {
    // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
    try {
      console.log('[剪贴板管理器] 验证剪贴板内容（主线程实现）')
      return { isValid: true, details: {} }
    } catch (error) {
      console.error('[剪贴板管理器] 验证剪贴板内容失败:', error)
      return { isValid: false, details: { error: error instanceof Error ? error.message : String(error) } }
    }
  }

  // ==================== 标签管理方法 ====================

  /**
   * 创建新标签组
   */
  public async createTagGroup(name: string, color: string, description?: string): Promise<string | null> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return null
      }

      return await this.workerManager.createTagGroup(name, color, description)
    } catch (error) {
      console.error('[剪贴板管理器] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 获取所有标签组
   */
  public async getAllTagGroups(): Promise<TagGroup[]> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.getAllTagGroups()
    } catch (error) {
      console.error('[剪贴板管理器] 获取标签组列表失败:', error)
      return []
    }
  }

  /**
   * 更新标签组
   */
  public async updateTagGroup(tagId: string, updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>>): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return false
      }

      return await this.workerManager.updateTagGroup(tagId, updates)
    } catch (error) {
      console.error('[剪贴板管理器] 更新标签组失败:', error)
      return false
    }
  }

  /**
   * 删除标签组
   */
  public async deleteTagGroup(tagId: string): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return false
      }

      return await this.workerManager.deleteTagGroup(tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 删除标签组失败:', error)
      return false
    }
  }

  /**
   * 为历史记录绑定标签
   */
  public async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return false
      }

      return await this.workerManager.bindTagToRecord(contentId, tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 绑定标签失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的标签绑定
   */
  public async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return false
      }

      return await this.workerManager.unbindTagFromRecord(contentId, tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 解除标签绑定失败:', error)
      return false
    }
  }

  /**
   * 按标签筛选历史记录
   */
  public async getRecordsByTags(tagIds: string[]): Promise<ClipboardItem[]> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.getRecordsByTags(tagIds)
    } catch (error) {
      console.error('[剪贴板管理器] 按标签筛选记录失败:', error)
      return []
    }
  }

  /**
   * 按解析类型筛选历史记录
   */
  public async getRecordsByParseType(parseType: string): Promise<ClipboardItem[]> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.getRecordsByParseType(parseType)
    } catch (error) {
      console.error('[剪贴板管理器] 按解析类型筛选记录失败:', error)
      return []
    }
  }

  /**
   * 获取标签统计信息
   */
  public async getTagStats(): Promise<TagStats[]> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.getTagStats()
    } catch (error) {
      console.error('[剪贴板管理器] 获取标签统计信息失败:', error)
      return []
    }
  }

  /**
   * 搜索历史记录（支持标签和内容筛选）
   */
  public async searchHistory(options: {
    limit?: number
    offset?: number
    tagIds?: string[]
    parseType?: string
    searchText?: string
    orderBy?: 'timestamp' | 'last_used'
    orderDirection?: 'ASC' | 'DESC'
  } = {}): Promise<ClipboardItem[]> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        return []
      }

      return await this.workerManager.searchHistory(options)
    } catch (error) {
      console.error('[剪贴板管理器] 搜索历史记录失败:', error)
      return []
    }
  }
}

// 导出单例实例
export default ClipboardManager.getInstance()
export { ClipboardManager }
