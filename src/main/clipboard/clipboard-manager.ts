import { BaseWorkerManager } from '../workers/base-worker-manager'
import { NotificationService } from '../services/notification-service'
import databaseService from '../services/database-service'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import type {
  ClipboardContent,
  ClipboardManagerConfig
} from '../types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'


import logger from '../utils/logger'
import clipboardManagerWorkerPath from '../workers/clipboard-manager.worker?modulePath'

/**
 * 剪贴板管理器（主线程接口）
 * 通过 Worker Thread 执行剪贴板管理操作以提升性能
 */
class ClipboardManager extends BaseWorkerManager {
  private static instance: ClipboardManager

  private constructor() {
    const config: WorkerConfig = {
      name: 'ClipboardManager',
      workerPath: clipboardManagerWorkerPath
    }
    super(config)

    // 设置事件监听器
    this.setupEventListeners()
  }

  /**
   * 设置事件监听器
   */
  private setupEventListeners(): void {
    // 监听剪贴板变化事件
    this.on('clipboard-changed', (data: { timestamp: number }) => {
      // 通知前端剪贴板历史更新
      this.notifyFrontendUpdate('clipboard-changed', data)
    })

    // 监听历史记录更新事件
    this.on('history-updated', (_: { timestamp: number, source?: string }) => {
      // 通知前端剪贴板历史更新
      // this.notifyFrontendUpdate('history-updated', data)
    })

    // 监听清理检查请求事件
    this.on('cleanup-check-required', (data: { reason: string; maxHistorySize: number }) => {
      // 转发给清理器处理
      clipboardHistoryCleaner.emit('cleanup-check-required', data)
    })

    // 监听清理完成事件，直接更新缓存
    clipboardHistoryCleaner.on('cleanup-completed', (data: { stats: any, deletedRecordIds?: string[] }) => {
      // 直接通知 clipboard-manager worker 更新缓存
      if (data.deletedRecordIds && data.deletedRecordIds.length > 0) {
        this.notifyWorkerCacheUpdate(data.deletedRecordIds)
      }
    })
  }

  /**
   * 通知前端剪贴板历史更新
   */
  private notifyFrontendUpdate(eventType: string, data: any): void {
    try {
      NotificationService.notifyClipboardHistoryUpdate({
        eventType,
        timestamp: Date.now(),
        ...data
      })
    } catch (error) {
      logger.error('[剪贴板管理器] 通知前端更新失败:', error)
    }
  }

  /**
   * 通知 Worker 更新缓存
   * 直接调用 Worker 的缓存同步方法
   */
  private async notifyWorkerCacheUpdate(deletedIds: string[]): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('syncDeletedRecordsByIds', deletedIds)
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 通知 Worker 缓存更新失败:', error)
    }
  }

  /**
   * 重写基类方法来处理来自 Worker 的请求
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    this.handleWorkerCall(request)
  }

  /**
   * 处理 Worker 调用
   */
  private async handleWorkerCall(request: WorkerRequest): Promise<void> {
    try {
      let result: any

      // 检查是否是数据库操作
      if (request.method.startsWith('database-')) {
        const operation = request.method.replace('database-', '')
        result = await databaseService.executeOperation(operation, request.params)
      } else {
        throw new Error(`未知的调用方法: ${request.method}`)
      }

      // 发送成功响应
      this.worker?.postMessage({
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: true,
        data: result,
        timestamp: Date.now()
      })
      // console.log('send response to worker', new Date().toDateString())
    } catch (error) {
      // 发送错误响应
      this.worker?.postMessage({
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        timestamp: Date.now()
      })
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

  // ==================== 剪贴板操作方法 ====================



  public async restoreClipboardContent(params: { content: ClipboardContent }): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()
      return await this.call('restoreClipboardContent', params)
    } catch (error) {
      logger.error('[剪贴板管理器] 还原剪贴板失败:', error)
      return false
    }
  }



  public async restoreClipboardContentAsText(params: { content: ClipboardContent }): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()
      return await this.call('restoreClipboardContentAsText', params)
    } catch (error) {
      logger.error('[剪贴板管理器] 还原剪贴板为纯文本失败:', error)
      return false
    }
  }

  /**
   * 确保 Worker 已初始化
   */
  private async ensureWorkerInitialized(): Promise<void> {
    if (!this.isInitialized) {
      await this.start()
      // isInitialized 已在 start() 方法中设置，无需重复设置
    }
  }

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    await this.ensureWorkerInitialized()

    // 从store中获取配置
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)

    const finalConfig: Partial<ClipboardManagerConfig> = {
      maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false,
      ...config
    }

    await this.call('initialize', finalConfig)
  }

  /**
   * 开始监听剪贴板变化
   */
  public async startListening(): Promise<void> {
    await this.ensureWorkerInitialized()

    await this.call('startListening')
  }

  /**
   * 停止监听剪贴板变化
   */
  public async stopListening(): Promise<void> {
    if (this.isRunning()) {
      await this.call('stopListening')
    }
  }

  /**
   * 同步删除记录状态到 Worker
   */
  public async syncDeletedRecords(deletedIds: string[]): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('syncDeletedRecords', deletedIds)
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 同步删除记录状态失败:', error)
    }
  }

  /**
   * 获取配置
   */
  public async getConfig(): Promise<ClipboardManagerConfig> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('getConfig')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取配置失败:', error)
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

      await this.call('updateConfig', newConfig)
    } catch (error) {
      logger.error('[剪贴板管理器] 更新配置失败:', error)
    }
  }

  /**
   * 从store更新maxHistorySize配置
   */
  public async updateMaxHistorySizeFromStore(): Promise<void> {
    try {
      const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
      await this.updateConfig({ maxHistorySize })
    } catch (error) {
      logger.error('[剪贴板管理器] 从store更新maxHistorySize失败:', error)
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
      })
    } catch (error) {
      logger.error('[剪贴板管理器] 更新清理器配置失败:', error)
    }
  }

  /**
   * 获取监听状态
   */
  public async isListeningActive(): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('isListeningActive')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取监听状态失败:', error)
      return false
    }
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   */
  public async syncClipboardHash(): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      await this.call('syncClipboardHash')
    } catch (error) {
      logger.error('[剪贴板管理器] 同步剪贴板哈希值失败:', error)
    }
  }

  /**
   * 系统唤醒后的恢复处理
   * 确保剪贴板管理器在系统休眠唤醒后正常工作
   */
  public async handleSystemResume(): Promise<void> {
    try {
      logger.info('[剪贴板管理器] 开始处理系统唤醒恢复')

      // 确保Worker已初始化
      await this.ensureWorkerInitialized()

      // 重置错误状态
      await this.call('resetErrorState')

      // 同步剪贴板哈希值
      await this.syncClipboardHash()

      // 如果之前在监听，重新启动监听
      const wasListening = await this.isListeningActive()
      if (!wasListening) {
        await this.startListening()
      }

      logger.info('[剪贴板管理器] 系统唤醒恢复完成')
    } catch (error) {
      logger.error('[剪贴板管理器] 系统唤醒恢复失败:', error)
    }
  }

  /**
   * 获取当前剪贴板内容（不触发监控）
   */
  public async getCurrentClipboard(): Promise<ClipboardContent> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('getCurrentClipboard')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取当前剪贴板内容失败:', error)
      return {}
    }
  }

  /**
   * 从数据库还原剪贴板内容（完整还原）
   */
  public async restoreClipboardFromDatabase(itemId: string): Promise<{ success: boolean; message?: string }> {
    try {
      await this.ensureWorkerInitialized()
      return await this.call('restoreClipboardFromDatabase', itemId)
    } catch (error) {
      logger.error('[剪贴板管理器] 从数据库还原剪贴板失败:', error)
      return { success: false, message: '还原剪贴板失败' }
    }
  }

  /**
   * 从数据库还原剪贴板内容为纯文本
   */
  public async restoreClipboardFromDatabaseAsText(itemId: string): Promise<{ success: boolean; message?: string }> {
    try {
      await this.ensureWorkerInitialized()
      return await this.call('restoreClipboardFromDatabaseAsText', itemId)
    } catch (error) {
      logger.error('[剪贴板管理器] 从数据库还原剪贴板为纯文本失败:', error)
      return { success: false, message: '还原剪贴板为纯文本失败' }
    }
  }



  // ==================== 标签管理方法 ====================




}

// 导出单例实例
export default ClipboardManager.getInstance()
export { ClipboardManager }
