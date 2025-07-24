import { BaseWorkerManager } from '../workers/base-worker-manager'
import { NotificationService } from '../services/notification-service'
import databaseService from '../services/database-service'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import type { CleanupConfig, CleanupStats } from '../types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import clipboardHistoryCleanerWorkerPath from '../workers/clipboard-history-cleaner.worker?modulePath'
import logger from '../utils/logger'

/**
 * 剪贴板历史记录清理器（主线程接口）
 * 通过 Worker Thread 执行清理操作以提升性能
 */
class ClipboardHistoryCleaner extends BaseWorkerManager {
  private static instance: ClipboardHistoryCleaner

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
    // 监听清理检查请求（来自剪贴板管理器）
    this.on('cleanup-check-required', (data: { reason: string; maxHistorySize: number }) => {
      if (data.reason === 'new-record-added' && data.maxHistorySize > 0) {
        // 异步执行基于数量的清理检查
        this.checkCountBasedCleanup(data.maxHistorySize).catch((error: any) => {
          logger.error('[清理器管理器] 基于数量的清理检查失败:', error)
        })
      }
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
      // 需要导入 app 来获取 userDataPath
      const { app } = require('electron')
      const userDataPath = app.getPath('userData')
      await this.call('initialize', userDataPath)
      this.updateConfigFromStore()
      this.isInitialized = true
    }
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
   * 检查是否需要基于数量的清理
   */
  public async checkCountBasedCleanup(maxHistorySize: number): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      await this.call('checkCountBasedCleanup', maxHistorySize)
    } catch (error) {
      logger.error('[清理器管理器] 基于数量的清理检查失败:', error)
    }
  }

  /**
   * 从 store 更新配置
   */
  public updateConfigFromStore(): void {
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)

    const config = {
      maxKeepTime,
      maxHistorySize,
      enableCountBasedCleanup: true
    }

    // 更新 Worker 配置
    if (this.isRunning()) {
      this.call('updateConfig', config).catch((error) => {
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
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    return {
      maxKeepTime,
      maxHistorySize,
      cleanupInterval: 60 * 60 * 1000,
      enableAutoCleanup: maxKeepTime !== -1,
      enableCountBasedCleanup: true // 默认启用基于数量的清理
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
