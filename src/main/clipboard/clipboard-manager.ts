import { BaseWorkerManager } from '../workers/base-worker-manager'
import { NotificationService } from '../services/notification-service'
import databaseService from '../services/database-service'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import type { ClipboardContent, ClipboardManagerConfig } from '../types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import { app } from 'electron'

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
    this.on('history-updated', (_: { timestamp: number; source?: string }) => {
      // 通知前端剪贴板历史更新
      // this.notifyFrontendUpdate('history-updated', data)
    })

    // 监听单条记录OCR更新事件
    this.on(
      'record-ocr-updated',
      (data: { contentId: string; ocrText: string; timestamp: number }) => {
        // 通知前端单条记录OCR更新
        NotificationService.notifyRecordOCRUpdate(data)
      }
    )

    // 监听清理检查请求事件
    this.on('cleanup-check-required', (data: { reason: string; maxHistorySize: number }) => {
      // 转发给清理器处理
      clipboardHistoryCleaner.emit('cleanup-check-required', data)
    })

    // 监听清理完成事件，直接更新缓存
    clipboardHistoryCleaner.on(
      'cleanup-completed',
      (data: { stats: any; deletedRecordIds?: string[] }) => {
        logger.info('[剪贴板管理器] 收到清理完成事件:', data.stats)
        // 直接通知 clipboard-manager worker 更新缓存
        if (data.deletedRecordIds && data.deletedRecordIds.length > 0) {
          this.notifyWorkerCacheUpdate(data.deletedRecordIds)
        }
      }
    )
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
   * 安全封装的 Worker 调用
   */
  private async safeCall<T>(
    method: string,
    errorMessage: string,
    fallback: T,
    params?: unknown
  ): Promise<T> {
    try {
      return await this.call<T>(method, params)
    } catch (error) {
      logger.error(`[剪贴板管理器] ${errorMessage}:`, error)
      return fallback
    }
  }

  /**
   * 获取默认配置
   */
  private getDefaultConfig(): ClipboardManagerConfig {
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    const enableLocalOCR = store.get(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false)
    return {
      maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false,
      enableLocalOCR
    }
  }

  /**
   * 通知 Worker 更新缓存
   * 直接调用 Worker 的缓存同步方法
   */
  private async notifyWorkerCacheUpdate(deletedIds: string[]): Promise<void> {
    if (this.isRunning()) {
      await this.safeCall('syncDeletedRecordsByIds', '通知 Worker 缓存更新失败', undefined, deletedIds)
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
    return this.safeCall('restoreClipboardContent', '还原剪贴板失败', false, params)
  }

  public async restoreClipboardContentAsText(params: {
    content: ClipboardContent
  }): Promise<boolean> {
    return this.safeCall(
      'restoreClipboardContentAsText',
      '还原剪贴板为纯文本失败',
      false,
      params
    )
  }

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    // 启动 Worker
    if (!this.isInitialized) {
      await this.start()
    }

    // 从store中获取配置
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    const enableLocalOCR = store.get(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false)

    const finalConfig: Partial<ClipboardManagerConfig> = {
      maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false,
      enableLocalOCR,
      ...config
    }

    // 需要导入 app 来获取 userDataPath、isPackaged 和 appPath
    const userDataPath = app.getPath('userData')
    const isPackaged = app.isPackaged
    const appPath = app.getAppPath()

    await this.call('initialize', { config: finalConfig, userDataPath, isPackaged, appPath })
  }

  /**
   * 开始监听剪贴板变化
   */
  public async startListening(): Promise<void> {
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
    if (!this.isRunning()) {
      return
    }
    await this.safeCall('syncDeletedRecords', '同步删除记录状态失败', undefined, deletedIds)
  }

  /**
   * 获取配置
   */
  public async getConfig(): Promise<ClipboardManagerConfig> {
    return this.safeCall('getConfig', '获取配置失败', this.getDefaultConfig())
  }

  /**
   * 更新配置
   */
  public async updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    await this.safeCall('updateConfig', '更新配置失败', undefined, newConfig)
  }

  /**
   * 获取监听状态
   */
  public async isListeningActive(): Promise<boolean> {
    return this.safeCall('isListeningActive', '获取监听状态失败', false)
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   */
  public async syncClipboardHash(): Promise<void> {
    await this.safeCall('syncClipboardHash', '同步剪贴板哈希值失败', undefined)
  }

  /**
   * 系统唤醒后的恢复处理
   * 确保剪贴板管理器在系统休眠唤醒后正常工作
   */
  public async handleSystemResume(): Promise<void> {
    try {
      logger.info('[剪贴板管理器] 开始处理系统唤醒恢复')

      await this.safeCall('resetErrorState', '系统唤醒恢复期间重置错误状态失败', undefined)

      await this.syncClipboardHash()

      const wasListening = await this.isListeningActive()
      if (!wasListening) {
        try {
          await this.startListening()
        } catch (error) {
          logger.error('[剪贴板管理器] 系统唤醒恢复期间重新启动监听失败:', error)
        }
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
    return this.safeCall('getCurrentClipboard', '获取当前剪贴板内容失败', {} as ClipboardContent)
  }

  /**
   * 从数据库还原剪贴板内容（完整还原）
   */
  public async restoreClipboardFromDatabase(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    return this.safeCall(
      'restoreClipboardFromDatabase',
      '从数据库还原剪贴板失败',
      { success: false, message: '还原剪贴板失败' },
      itemId
    )
  }

  /**
   * 从数据库还原剪贴板内容为纯文本
   */
  public async restoreClipboardFromDatabaseAsText(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    return this.safeCall(
      'restoreClipboardFromDatabaseAsText',
      '从数据库还原剪贴板为纯文本失败',
      { success: false, message: '还原剪贴板为纯文本失败' },
      itemId
    )
  }

  /**
   * 解析剪贴板项目中的二维码并复制到剪贴板
   * @param itemId 剪贴板项目ID
   */
  public async parseQRCodeFromItem(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    return this.safeCall(
      'parseQRCodeFromItem',
      '解析二维码失败',
      { success: false, message: '解析二维码失败' },
      itemId
    )
  }

  // ==================== 标签管理方法 ====================
}

// 导出单例实例
export default ClipboardManager.getInstance()
export { ClipboardManager }
