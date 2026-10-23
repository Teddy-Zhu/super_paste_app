import { recordAIOCRUsage } from '../../../ocr/usage-service'
import { recordCopyStatistics } from '../../../statistics/copy-statistics-service'
import type { CopyStatisticsEvent } from '../../../../shared/clipboard-statistics'
import { DEFAULT_OCR_MODEL_ID, getOCRModel, type OCRModelId } from '../../../../shared/ocr-models'
import {
  AI_OCR_REQUEST_TIMEOUT_MS,
  createDefaultAIOCRConfigs,
  type AIOCRUsageEntry,
  type OCRTaskStatus
} from '../../../../shared/ai-ocr'
import { getOCRModelStore } from '../../../ocr/model-service'
import { getInstalledOCRModel } from '../../../ocr/model-store'
import path from 'node:path'
import { BaseWorkerManager } from '../../../workers/base-worker-manager'
import { NotificationService } from '../../../services/notification-service'
import store from '../../../config/store'
import { STORE_KEY } from '../../../config/common'
import type { ClipboardContent, ClipboardManagerConfig } from '../types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from '../../../types/worker-types'
import { app } from 'electron'

import logger from '../../../utils/logger'
import clipboardManagerWorkerPath from '../worker/clipboard-manager.worker?modulePath'
import ocrWorkerPath from '../../../ocr/ocr.worker?modulePath'

/**
 * 剪贴板管理器（主线程接口）
 * 通过 Worker Thread 执行剪贴板管理操作以提升性能
 */
class ClipboardManager extends BaseWorkerManager {
  private static instance: ClipboardManager
  private configUpdates: Promise<void> = Promise.resolve()
  private ocrPreparation: Promise<void> | null = null
  private ocrPreparationRequested = false
  private modelMaintenance = new Set<OCRModelId>()

  private constructor() {
    const config: WorkerConfig = {
      name: 'ClipboardManager',
      workerPath: clipboardManagerWorkerPath,
      options: { workerData: { ocrWorkerPath } }
    }
    super(config)

    // 设置事件监听器
    this.setupEventListeners()
  }

  /**
   * 设置事件监听器
   */
  private setupEventListeners(): void {
    this.on('copy-statistics', (event: CopyStatisticsEvent) => {
      try {
        recordCopyStatistics(event)
      } catch (error) {
        logger.error('[复制统计] 累计失败:', error)
      }
    })
    // 监听剪贴板变化事件
    this.on('clipboard-changed', (data: { timestamp: number }) => {
      // 通知前端剪贴板历史更新
      this.notifyFrontendUpdate('clipboard-changed', data)
    })

    // 监听历史记录更新事件
    this.on('history-updated', () => {
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

    this.on('ocr-requested', () => {
      void this.prepareQueuedOCR().catch((error) => logger.error('[OCR] 准备后台识别失败:', error))
    })
    this.on('ocr:task-status', (status: OCRTaskStatus) => {
      NotificationService.notifyFrontend('ocr:task-status', status)
    })
    this.on('ai-ocr-usage', (entry: AIOCRUsageEntry) => {
      try {
        recordAIOCRUsage(entry)
      } catch (error) {
        logger.error('[OCR] 保存 AI 用量失败:', error)
      }
    })
  }

  /**
   * 通知前端剪贴板历史更新
   */
  private notifyFrontendUpdate(
    eventType: string,
    data: { timestamp: number; source?: string }
  ): void {
    try {
      NotificationService.notifyClipboardHistoryUpdate({
        eventType,
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
      logger.error(`[剪贴板管理器] ${errorMessage}: `, error)
      return fallback
    }
  }

  /**
   * 获取默认配置
   */
  private getDefaultConfig(): ClipboardManagerConfig {
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    return {
      maxHistorySize,
      enableAutoSave: false,
      ...this.getOCRConfigFromStore()
    }
  }

  private getOCRConfigFromStore(): Pick<
    ClipboardManagerConfig,
    'enableLocalOCR' | 'ocrMode' | 'ocrLocalModel' | 'ocrRemoteConfig' | 'ocrAIConfigs'
  > {
    return {
      enableLocalOCR: store.get(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false),
      ocrMode: store.get(STORE_KEY.SETTINGS_OCR_MODE, 'local'),
      ocrLocalModel: store.get(STORE_KEY.SETTINGS_OCR_LOCAL_MODEL, DEFAULT_OCR_MODEL_ID),
      ocrAIConfigs: store.get(STORE_KEY.SETTINGS_OCR_AI_CONFIGS, createDefaultAIOCRConfigs()),
      ocrRemoteConfig: {
        url: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_URL, ''),
        extraParams: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_EXTRA_PARAMS, '{}'),
        imageFieldName: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_IMAGE_FIELD_NAME, 'image'),
        requestFormat: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_REQUEST_FORMAT, 'json'),
        resultPath: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_RESULT_PATH, 'data.result')
      }
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
   * 注意：Worker 现在应该直接访问数据库，此方法已弃用
   */
  private async handleWorkerCall(request: WorkerRequest): Promise<void> {
    try {
      // Worker 应该直接访问数据库，不应再通过主线程调用
      // 如果收到此类请求，记录警告
      logger.warn('[剪贴板管理器] Worker 尝试通过主线程调用数据库，这不应该发生:', request.method)

      // 发送错误响应
      this.worker?.postMessage({
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: false,
        error: 'Worker 应直接访问数据库',
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
    return this.safeCall('restoreClipboardContentAsText', '还原剪贴板为纯文本失败', false, params)
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
    const ocrConfig = this.getOCRConfigFromStore()
    const finalConfig: Partial<ClipboardManagerConfig> = {
      maxHistorySize,
      enableAutoSave: false,
      ...ocrConfig,
      ...config,
      ocrRemoteConfig: {
        ...ocrConfig.ocrRemoteConfig,
        ...(config?.ocrRemoteConfig || {})
      }
    }

    const userDataPath = app.getPath('userData')
    await this.call('initialize', { config: finalConfig, userDataPath })
    this.prepareAutoOCRModel().catch((error) => logger.error('[OCR] 自动准备模型失败:', error))
  }

  private async prepareOCRModel(modelId: OCRModelId, verify = true): Promise<boolean> {
    const root = path.join(app.getPath('userData'), 'ocr-models')
    if (await getInstalledOCRModel(root, modelId, verify)) return false
    await getOCRModelStore().download(modelId)
    return true
  }

  private async prepareAutoOCRModel(): Promise<void> {
    if ((await this.getConfig()).enableLocalOCR) await this.prepareQueuedOCR()
  }

  private prepareQueuedOCR(): Promise<void> {
    this.ocrPreparationRequested = true
    if (this.ocrPreparation) return this.ocrPreparation
    this.ocrPreparation = (async () => {
      try {
        while (true) {
          this.ocrPreparationRequested = false
          const config = await this.getConfig()
          if (!config.enableLocalOCR && !(await this.call<boolean>('hasOCRJobs'))) return
          if (config.ocrMode === 'local') {
            if (this.modelMaintenance.has(config.ocrLocalModel)) return
            await this.prepareOCRModel(config.ocrLocalModel)
          }
          const current = await this.getConfig()
          if (current.ocrMode !== config.ocrMode || current.ocrLocalModel !== config.ocrLocalModel)
            continue
          await this.call('resumeOCR', undefined, AI_OCR_REQUEST_TIMEOUT_MS + 5000)
          if (this.ocrPreparationRequested) continue
          const latest = await this.getConfig()
          if (latest.ocrMode !== config.ocrMode || latest.ocrLocalModel !== config.ocrLocalModel)
            continue
          return
        }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        logger.error('[OCR] 准备后台识别失败:', message)
        const current = await this.getConfig()
        if (current.ocrMode !== 'local' || !this.modelMaintenance.has(current.ocrLocalModel))
          await this.call('failPendingOCR', message)
      } finally {
        this.ocrPreparation = null
      }
    })()
    return this.ocrPreparation
  }

  public parseOcrFromItem(itemId: string): Promise<{ success: boolean; message?: string }> {
    return this.call('parseOcrFromItem', itemId)
  }

  public async maintainOCRModel(id: unknown, action: 'delete' | 'redownload'): Promise<void> {
    const model = getOCRModel(id)
    if (this.modelMaintenance.has(model.id)) throw new Error('模型正在维护，请稍后重试')
    this.modelMaintenance.add(model.id)
    try {
      await this.call('suspendOCRModel', model.id, AI_OCR_REQUEST_TIMEOUT_MS + 5000)
      if (action === 'delete') {
        const config = await this.getConfig()
        if (config.ocrMode === 'local' && config.ocrLocalModel === model.id) {
          await this.call('failPendingOCR', '本地模型已删除，请重新提交 OCR')
          if (config.enableLocalOCR) {
            await this.updateConfig({ enableLocalOCR: false })
            store.set(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false, {
              source: { kind: 'main', id: 'model-maintenance' }
            })
          }
        }
        await getOCRModelStore().remove(model.id)
      } else await getOCRModelStore().redownload(model.id)
    } finally {
      this.modelMaintenance.delete(model.id)
      await this.call('releaseOCRModel', model.id)
      void this.prepareQueuedOCR().catch((error) => logger.error('[OCR] 恢复后台识别失败:', error))
    }
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
  public updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    const update = this.configUpdates.then(async () => {
      await this.call<void>('updateConfig', newConfig)
      void this.prepareQueuedOCR().catch((error) => logger.error('[OCR] 更新后台识别失败:', error))
    })
    this.configUpdates = update.then(
      () => undefined,
      () => undefined
    )
    return update
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
