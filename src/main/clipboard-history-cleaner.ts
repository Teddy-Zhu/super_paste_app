import { BaseWorkerManager } from './workers/base-worker-manager'
import { NotificationService } from './services/notification-service'
import databaseService from './services/database-service'
import store from './store'
import { STORE_KEY } from './common'
import type { CleanupConfig, CleanupStats } from './types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from './types/worker-types'
import clipboardHistoryCleanerWorkerPath from './workers/clipboard-history-cleaner.worker?modulePath'
import logger from './utils/logger'

/**
 * 剪贴板历史记录清理器（主线程接口）
 * 通过 Worker Thread 执行清理操作以提升性能
 */
class ClipboardHistoryCleaner extends BaseWorkerManager {
  private static instance: ClipboardHistoryCleaner
  private memoryDataSyncCallback: ((deletedIds: string[]) => void) | null = null

  private constructor() {
    const config: WorkerConfig = {
      name: 'ClipboardHistoryCleaner',
      workerPath: clipboardHistoryCleanerWorkerPath
    }
    super(config)



    // 设置事件监听器
    this.setupEventListeners()
  }

  /**
   * 设置事件监听器
   */
  private setupEventListeners(): void {
    // 监听内存同步请求
    this.on('memory-sync-required', (data: { deletedIds: string[] }) => {
      if (this.memoryDataSyncCallback) {
        this.memoryDataSyncCallback(data.deletedIds)
      }
    })

    // 监听清理完成事件
    this.on('cleanup-completed', (_: { stats: CleanupStats }) => {
      // 清理完成，可以在这里添加其他逻辑
    })

    // 监听清理错误事件
    this.on('cleanup-error', (data: { error: string }) => {
      logger.error('[清理器管理器] 清理错误:', data.error)
    })
  }

  /**
   * 重写基类方法来处理来自 Worker 的请求
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    this.handleDatabaseCall(request)
  }

  /**
   * 处理数据库调用
   */
  private async handleDatabaseCall(request: WorkerRequest): Promise<void> {
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
  public static getInstance(): ClipboardHistoryCleaner {
    if (!ClipboardHistoryCleaner.instance) {
      ClipboardHistoryCleaner.instance = new ClipboardHistoryCleaner()
    }
    return ClipboardHistoryCleaner.instance
  }

  /**
   * 确保 Worker 已初始化
   */
  private async ensureWorkerInitialized(): Promise<void> {
    if (!this.isInitialized) {
      await this.start()
      await this.call('initialize')
      this.updateConfigFromStore()
      this.isInitialized = true
    }
  }

  /**
   * 设置内存数据同步回调函数
   * 当清理器删除数据库记录时，会调用此回调来同步删除内存中的数据
   */
  public setMemoryDataSyncCallback(callback: (deletedIds: string[]) => void): void {
    this.memoryDataSyncCallback = callback
    this.ensureWorkerInitialized()
      .then(() => {
        // 通知 Worker 回调已设置
        this.call('setMemoryDataSyncCallback').catch(error => {
          logger.error('[清理器管理器] 设置内存同步回调失败:', error)
        })

      })
      .catch((error) => {
        logger.error('[历史记录清理器] 设置内存同步回调失败:', error)
      })
  }

  /**
   * 通知前端剪贴板历史已更新
   */
  private notifyFrontendHistoryUpdated(): void {
    try {
      NotificationService.notifyClipboardHistoryUpdate({
        timestamp: Date.now(),
        source: 'cleanup'
      })
    } catch (error) {
      logger.error('[历史记录清理器] 通知前端失败:', error)
    }
  }

  /**
   * 启动自动清理
   */
  public async startAutoCleanup(): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      await this.call('startAutoCleanup')
    } catch (error) {
      logger.error('[历史记录清理器] 启动自动清理失败:', error)
    }
  }

  /**
   * 停止自动清理
   */
  public async stopAutoCleanup(): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('stopAutoCleanup')
      }
    } catch (error) {
      logger.error('[历史记录清理器] 停止自动清理失败:', error)
    }
  }

  /**
   * 执行清理操作
   */
  public async performCleanup(): Promise<CleanupStats> {
    try {
      await this.ensureWorkerInitialized()

      const stats = await this.call('performCleanup')

      // 如果有记录被删除，通知前端更新
      if (stats.deletedRecords > 0) {
        this.notifyFrontendHistoryUpdated()
      }

      return stats
    } catch (error) {
      logger.error('[历史记录清理器] 执行清理操作失败:', error)
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }
  }

  /**
   * 从 store 更新配置
   */
  public updateConfigFromStore(): void {
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)

    // 更新 Worker 配置
    if (this.isRunning()) {
      this.call('updateConfig', { maxKeepTime }).catch((error) => {
        logger.error('[历史记录清理器] 更新配置失败:', error)
      })
    }
  }

  /**
   * 获取当前配置
   */
  public getConfig(): CleanupConfig {
    if (this.isRunning()) {
      // 异步获取配置，但为了兼容性返回默认配置
      this.call('getConfig')
        .catch((error) => {
          logger.error('[历史记录清理器] 获取配置失败:', error)
        })
    }

    // 返回默认配置以保持兼容性
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)
    return {
      maxKeepTime,
      cleanupInterval: 60 * 60 * 1000,
      enableAutoCleanup: maxKeepTime !== -1
    }
  }

  /**
   * 销毁清理器（应用退出时调用）
   */
  public async destroy(): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('destroy')
      }
      await this.stop()
    } catch (error) {
      logger.error('[历史记录清理器] 销毁清理器失败:', error)
    }
  }
}

// 导出单例实例
export default ClipboardHistoryCleaner.getInstance()
export { ClipboardHistoryCleaner }
