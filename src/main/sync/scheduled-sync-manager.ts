import { BaseWorkerManager } from '../workers/base-worker-manager'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import type { SyncResult, ScheduledSyncConfig, ScheduledSyncStatus } from './types'
import logger from '../utils/logger'
import scheduledSyncWorkerPath from './scheduled-sync.worker?modulePath'
import { BrowserWindow } from 'electron'

/**
 * 定时同步任务 Worker 管理器
 * 负责管理定时同步任务 Worker 的生命周期和通信
 */
export class ScheduledSyncManager extends BaseWorkerManager {
  private static instance: ScheduledSyncManager

  constructor() {
    const config: WorkerConfig = {
      name: '定时同步任务管理器',
      workerPath: scheduledSyncWorkerPath,
      options: {
        workerData: {}
      }
    }
    super(config)
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ScheduledSyncManager {
    if (!ScheduledSyncManager.instance) {
      ScheduledSyncManager.instance = new ScheduledSyncManager()
    }
    return ScheduledSyncManager.instance
  }

  /**
   * 初始化定时同步
   * 总是尝试初始化 Worker，即使配置不完整也确保基础功能可用
   */
  public async initialize(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      // 确保 Worker 进程启动
      if (!this.isRunning()) {
        await this.start()
      }

      // 尝试初始化 Worker（即使配置不完整也应该成功）
      const result = await this.call<boolean>('initialize', config)

      if (result) {
        logger.info(`[${this.config.name}] Worker 基础功能初始化成功`)

        // 只有在配置完整且启用时才尝试启动定时同步
        if (config && config.enabled && this.isConfigComplete(config)) {
          logger.info(`[${this.config.name}] 配置完整且已启用，尝试启动定时同步`)
          try {
            const started = await this.startScheduledSync()
            if (started) {
              logger.info(`[${this.config.name}] 定时同步已自动启动`)
            } else {
              logger.warn(`[${this.config.name}] 定时同步自动启动失败，但 Worker 基础功能正常`)
            }
          } catch (startError) {
            logger.warn(
              `[${this.config.name}] 定时同步启动异常，但 Worker 基础功能正常:`,
              startError
            )
          }
        } else {
          logger.info(`[${this.config.name}] 配置不完整或未启用，仅 Worker 基础功能可用`)
        }
      } else {
        logger.warn(`[${this.config.name}] Worker 初始化失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 初始化过程异常:`, error)

      // 即使出现异常，也尝试确保 Worker 进程可用
      try {
        if (!this.isRunning()) {
          await this.start()
          logger.info(`[${this.config.name}] Worker 进程启动成功，但配置初始化失败`)
          return true // Worker 进程可用，即使配置有问题
        }
      } catch (startError) {
        logger.error(`[${this.config.name}] Worker 进程启动也失败:`, startError)
      }

      return false
    }
  }

  /**
   * 检查配置是否完整
   */
  private isConfigComplete(config: ScheduledSyncConfig): boolean {
    if (!config || !config.userDataPath || !config.providerType) {
      return false
    }

    if (!config.syncInterval || config.syncInterval <= 0) {
      return false
    }

    if (!config.providerConfig) {
      return false
    }

    // 根据提供者类型检查具体配置
    if (config.providerType === 'webdav') {
      const webdavConfig = config.providerConfig
      return !!(
        webdavConfig.serverUrl &&
        webdavConfig.username &&
        webdavConfig.password &&
        webdavConfig.syncDirection
      )
    }

    return true
  }

  /**
   * 启动定时同步
   */
  public async startScheduledSync(): Promise<boolean> {
    try {
      const result = await this.call<boolean>('startScheduledSync')

      if (result) {
        logger.info(`[${this.config.name}] 定时同步已启动`)
      } else {
        logger.warn(`[${this.config.name}] 启动定时同步失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 启动定时同步失败:`, error)
      return false
    }
  }

  /**
   * 停止定时同步
   */
  public async stopScheduledSync(): Promise<boolean> {
    try {
      const result = await this.call<boolean>('stopScheduledSync')

      if (result) {
        logger.info(`[${this.config.name}] 定时同步已停止`)
      } else {
        logger.warn(`[${this.config.name}] 停止定时同步失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 停止定时同步失败:`, error)
      return false
    }
  }

  /**
   * 执行手动同步
   */
  public async manualSync(): Promise<SyncResult> {
    try {
      const result = await this.call<SyncResult>('manualSync')

      if (result.success) {
        logger.info(`[${this.config.name}] 手动同步完成:`, result.message)
      } else {
        logger.warn(`[${this.config.name}] 手动同步失败:`, result.message)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 执行手动同步失败:`, error)
      return {
        success: false,
        message: error instanceof Error ? error.message : '未知错误',
        errors: [error instanceof Error ? error.message : '未知错误']
      }
    }
  }

  /**
   * 使用指定配置执行手动同步
   */
  public async manualSyncWithConfig(config: ScheduledSyncConfig): Promise<SyncResult> {
    try {
      const result = await this.call<SyncResult>('manualSyncWithConfig', config)

      if (result.success) {
        logger.info(`[${this.config.name}] 使用指定配置手动同步完成:`, result.message)
      } else {
        logger.warn(`[${this.config.name}] 使用指定配置手动同步失败:`, result.message)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 使用指定配置执行手动同步失败:`, error)
      return {
        success: false,
        message: error instanceof Error ? error.message : '未知错误',
        errors: [error instanceof Error ? error.message : '未知错误']
      }
    }
  }

  /**
   * 测试同步连接
   */
  public async testConnection(): Promise<boolean> {
    try {
      const result = await this.call<boolean>('testConnection')

      if (result) {
        logger.info(`[${this.config.name}] 连接测试成功`)
      } else {
        logger.warn(`[${this.config.name}] 连接测试失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 测试连接失败:`, error)
      return false
    }
  }

  /**
   * 使用指定配置测试同步连接
   */
  public async testConnectionWithConfig(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      // 确保worker已启动（通常应该在initialize时已启动）
      if (!this.isRunning()) {
        await this.start()
      }

      const result = await this.call<boolean>('testConnectionWithConfig', config)

      if (result) {
        logger.info(`[${this.config.name}] 使用指定配置连接测试成功`)
      } else {
        logger.warn(`[${this.config.name}] 使用指定配置连接测试失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 使用指定配置测试连接失败:`, error)
      return false
    }
  }

  /**
   * 获取同步状态
   */
  public async getSyncStatus(): Promise<ScheduledSyncStatus | null> {
    try {
      const result = await this.call<ScheduledSyncStatus>('getSyncStatus')
      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 获取同步状态失败:`, error)
      return null
    }
  }

  /**
   * 更新配置
   */
  public async updateConfig(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      const result = await this.call<boolean>('updateConfig', config)

      if (result) {
        logger.info(`[${this.config.name}] 配置更新成功`)
      } else {
        logger.warn(`[${this.config.name}] 配置更新失败`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 更新配置失败:`, error)
      return false
    }
  }

  /**
   * 销毁 Worker
   */
  public async destroy(): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('destroy')
        await this.stop()
      }
      logger.info(`[${this.config.name}] Worker 管理器已销毁`)
    } catch (error) {
      logger.error(`[${this.config.name}] 销毁 Worker 管理器失败:`, error)
    }
  }

  /**
   * 处理来自 Worker 的请求
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    try {
      switch (request.method) {
        default:
          logger.debug(`[${this.config.name}] 收到未处理的 Worker 请求:`, request.method)
      }
    } catch (error) {
      logger.error(`[${this.config.name}] 处理 Worker 请求失败:`, error)
      // 发送错误响应
      this.sendWorkerResponse(
        request.id,
        false,
        undefined,
        error instanceof Error ? error.message : '处理请求失败'
      )
    }
  }

  /**
   * 向 Worker 发送响应
   */
  private sendWorkerResponse(
    requestId: string,
    success: boolean,
    data?: unknown,
    error?: string
  ): void {
    if (!this.worker) {
      return
    }

    const response = {
      id: `response-${Date.now()}`,
      type: 'response',
      requestId: requestId,
      success: success,
      data: data,
      error: error,
      timestamp: Date.now()
    }

    this.worker.postMessage(response)
  }

  /**
   * 向所有窗口发送IPC事件
   */
  private sendIpcEventToAllWindows(channel: string, data?: unknown): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send(channel, data)
        }
      })
    } catch (error) {
      logger.error(`[${this.config.name}] 发送IPC事件失败:`, error)
    }
  }

  /**
   * 设置事件监听器
   */
  public setupEventListeners(): void {
    // 定时同步启动事件
    this.on('sync-scheduled-started', (data) => {
      logger.info(`[${this.config.name}] 定时同步已启动，间隔: ${data.interval} 分钟`)
    })

    // 定时同步停止事件
    this.on('sync-scheduled-stopped', () => {
      logger.info(`[${this.config.name}] 定时同步已停止`)
    })

    // 定时同步启动失败事件
    this.on('sync-scheduled-start-failed', (data) => {
      logger.error(`[${this.config.name}] 定时同步启动失败: ${data.reason}`)
    })

    // 同步开始事件
    this.on('sync-started', () => {
      logger.debug(`[${this.config.name}] 开始执行同步任务`)
      // 向前端发送同步开始事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'sync-started',
        timestamp: Date.now()
      })
    })

    // 同步完成事件
    this.on('sync-completed', (data) => {
      logger.info(`[${this.config.name}] 同步任务完成`)
      // 向前端发送同步完成事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'sync-completed',
        timestamp: data.timestamp,
        result: data.result,
        nextSyncTime: data.nextSyncTime
      })
    })

    // 同步失败事件
    this.on('sync-failed', (data) => {
      logger.warn(`[${this.config.name}] 同步任务失败:`, data.result?.message)
      // 向前端发送同步失败事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'sync-failed',
        timestamp: Date.now(),
        result: data.result
      })
    })

    // 同步错误事件
    this.on('sync-error', (data) => {
      logger.error(`[${this.config.name}] 同步任务发生错误:`, data.error)
    })

    // 手动同步事件
    this.on('manual-sync-started', () => {
      logger.debug(`[${this.config.name}] 开始执行手动同步`)
      // 向前端发送手动同步开始事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'manual-sync-started',
        timestamp: Date.now()
      })
    })

    this.on('manual-sync-completed', (data) => {
      logger.info(`[${this.config.name}] 手动同步完成`)
      // 向前端发送手动同步完成事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'manual-sync-completed',
        timestamp: data.timestamp,
        result: data.result
      })
    })

    this.on('manual-sync-failed', (data) => {
      logger.warn(`[${this.config.name}] 手动同步失败:`, data.result?.message)
      // 向前端发送手动同步失败事件
      this.sendIpcEventToAllWindows('sync-status-updated', {
        type: 'manual-sync-failed',
        timestamp: Date.now(),
        result: data.result
      })
    })

    this.on('manual-sync-error', (data) => {
      logger.error(`[${this.config.name}] 手动同步发生错误:`, data.error)
    })

    // 连接测试事件
    this.on('connection-test-success', () => {
      logger.debug(`[${this.config.name}] 连接测试成功`)
    })

    this.on('connection-test-failed', () => {
      logger.debug(`[${this.config.name}] 连接测试失败`)
    })

    this.on('connection-test-error', (data) => {
      logger.error(`[${this.config.name}] 连接测试发生错误:`, data.error)
    })

    // Worker 错误事件
    this.on('worker-error', (data) => {
      logger.error(`[${this.config.name}] Worker 发生错误:`, data.error)
    })

    // Worker 退出事件
    this.on('worker-exit', (data) => {
      logger.warn(`[${this.config.name}] Worker 已退出，退出码:`, data.code)
    })
  }
}

// 导出单例实例
export const scheduledSyncManager = ScheduledSyncManager.getInstance()
