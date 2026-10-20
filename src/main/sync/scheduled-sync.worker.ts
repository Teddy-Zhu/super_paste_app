import { parentPort } from 'node:worker_threads'

import { BaseWorker } from '../workers/base-worker'
import { SyncService } from './sync-service'

import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type { SyncResult, ScheduledSyncConfig } from '../sync/types'
import logger from '../utils/worker-logger'
import { SyncStatus } from '../sync/types'

/**
 * 定时同步任务 Worker
 * 负责在后台定期执行同步任务
 */
class ScheduledSyncWorker extends BaseWorker {
  private config: ScheduledSyncConfig | null = null
  private syncService: SyncService | null = null
  private syncTimer: NodeJS.Timeout | null = null
  private isRunning: boolean = false
  private isSyncing: boolean = false
  private lastSyncTime: number = 0
  private nextSyncTime: number = 0

  constructor() {
    super()
    // SyncService 将在 initialize 方法中创建
  }

  /**
   * 创建 SyncService 实例，注入通知回调（避免在 Worker 中 import electron）
   */
  private createSyncService(userDataPath: string): SyncService {
    return new SyncService(userDataPath, {
      onSyncRequiresRestart: (data) => {
        this.sendEvent('sync-requires-restart', data)
      }
    })
  }

  protected getWorkerName(): string {
    return '定时同步任务 Worker'
  }

  /**
   * 初始化定时同步 Worker
   * 总是初始化 Worker 基础功能，即使配置不完整也能成功初始化
   */
  public async initialize(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      // 初始化 Worker 基础功能（包括日志系统）
      this.initializeWorker({
        userDataPath: config?.userDataPath || '',
        logFileName: 'sync-worker.log',
        logScope: 'sync-worker'
      })

      // 保存配置（即使为空也保存）
      this.config = config

      // 检查配置是否完整且启用
      if (!this.isConfigValid(config)) {
        logger.info(`[${this.getWorkerName()}] 配置不完整或未启用，仅初始化 Worker 基础功能`)
        return true // Worker 基础功能初始化成功
      }

      // 尝试初始化同步服务
      try {
        // 创建同步服务实例
        this.syncService = this.createSyncService(config.userDataPath)

        // 初始化同步服务
        const initialized = await this.syncService.initialize(
          config.providerType,
          config.providerConfig
        )

        if (!initialized) {
          logger.warn(`[${this.getWorkerName()}] 同步服务初始化失败，但 Worker 基础功能正常`)
          // 销毁失败的同步服务实例
          if (this.syncService) {
            this.syncService.destroy()
            this.syncService = null
          }
          return true // Worker 基础功能仍然可用
        }

        logger.info(
          `[${this.getWorkerName()}] 完整初始化成功，同步间隔: ${config.syncInterval} 分钟`
        )
        return true
      } catch (syncError) {
        logger.warn(
          `[${this.getWorkerName()}] 同步服务初始化异常，但 Worker 基础功能正常:`,
          syncError
        )
        // 清理失败的同步服务
        if (this.syncService) {
          try {
            this.syncService.destroy()
          } catch (destroyError) {
            logger.debug(`[${this.getWorkerName()}] 清理同步服务时出错:`, destroyError)
          }
          this.syncService = null
        }
        return true // Worker 基础功能仍然可用
      }
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] Worker 基础功能初始化失败:`, error)
      return false // 只有在 Worker 基础功能都无法初始化时才返回 false
    }
  }

  /**
   * 检查配置是否有效且完整
   */
  private isConfigValid(config: ScheduledSyncConfig | null | undefined): boolean {
    if (!config) {
      return false
    }

    // 检查基础字段
    if (!config.enabled || !config.userDataPath || !config.providerType) {
      return false
    }

    // 检查同步间隔
    if (!config.syncInterval || config.syncInterval <= 0) {
      return false
    }

    // 检查提供者配置
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

    // 其他提供者类型的验证可以在这里添加
    return true
  }

  /**
   * 启动定时同步
   */
  public async startScheduledSync(): Promise<boolean> {
    if (!this.config || !this.config.enabled) {
      logger.warn(`[${this.getWorkerName()}] 配置未设置或未启用，无法启动定时同步`)
      return false
    }

    if (this.isRunning) {
      logger.info(`[${this.getWorkerName()}] 定时同步已在运行中`)
      return true
    }

    try {
      // 如果同步服务未初始化，先初始化
      if (!this.syncService) {
        logger.info(`[${this.getWorkerName()}] 同步服务未初始化，开始初始化...`)

        // 创建同步服务实例
        this.syncService = this.createSyncService(this.config.userDataPath)

        // 初始化同步服务
        const initialized = await this.syncService.initialize(
          this.config.providerType,
          this.config.providerConfig
        )

        if (!initialized) {
          logger.error(`[${this.getWorkerName()}] 同步服务初始化失败`)
          this.sendEvent('sync-scheduled-start-failed', {
            reason: '同步服务初始化失败',
            timestamp: Date.now()
          })
          return false
        }

        logger.info(`[${this.getWorkerName()}] 同步服务初始化成功`)
      }

      // 在启动定时同步前先测试 provider 连通性
      logger.info(`[${this.getWorkerName()}] 正在测试同步服务器连通性...`)
      const connectionTest = await this.testConnection()

      if (!connectionTest) {
        logger.error(`[${this.getWorkerName()}] 同步服务器连通性测试失败，无法启动定时同步`)
        this.sendEvent('sync-scheduled-start-failed', {
          reason: '同步服务器连通性测试失败',
          timestamp: Date.now()
        })
        return false
      }

      logger.info(`[${this.getWorkerName()}] 同步服务器连通性测试成功，启动定时同步`)

      this.isRunning = true
      this.scheduleNextSync()

      logger.info(
        `[${this.getWorkerName()}] 定时同步已启动，间隔: ${this.config.syncInterval} 分钟`
      )
      this.sendEvent('sync-scheduled-started', {
        interval: this.config.syncInterval,
        nextSyncTime: this.nextSyncTime
      })

      return true
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 启动定时同步失败:`, error)
      this.sendEvent('sync-scheduled-start-failed', {
        reason: error instanceof Error ? error.message : '未知错误',
        timestamp: Date.now()
      })
      return false
    }
  }

  /**
   * 停止定时同步
   */
  public stopScheduledSync(): boolean {
    try {
      if (this.syncTimer) {
        clearTimeout(this.syncTimer)
        this.syncTimer = null
      }

      this.isRunning = false
      this.isSyncing = false
      this.nextSyncTime = 0

      logger.info(`[${this.getWorkerName()}] 定时同步已停止`)
      this.sendEvent('sync-scheduled-stopped', {})

      return true
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 停止定时同步失败:`, error)
      return false
    }
  }

  /**
   * 安排下一次同步任务
   */
  private scheduleNextSync(): void {
    if (!this.config || !this.isRunning) {
      return
    }

    const intervalMs = this.config.syncInterval
    this.nextSyncTime = Date.now() + intervalMs

    this.syncTimer = setTimeout(async () => {
      await this.performSync()
    }, intervalMs)

    logger.debug(
      `[${this.getWorkerName()}] 已安排下次同步时间: ${new Date(this.nextSyncTime).toLocaleString()}`
    )
  }

  /**
   * 执行同步任务
   */
  private async performSync(): Promise<void> {
    // 检查是否应该停止（可能在等待期间被停止）
    if (!this.isRunning) {
      return
    }

    // 检查是否已经在同步中（防止重复执行）
    if (this.isSyncing) {
      logger.debug(`[${this.getWorkerName()}] 同步任务正在执行中，跳过本次调度`)
      this.scheduleNextSync() // 安排下一次同步
      return
    }

    if (this.isInErrorBackoff()) {
      logger.debug(`[${this.getWorkerName()}] 处于错误退避期间，跳过本次同步`)
      this.scheduleNextSync() // 安排下一次同步
      return
    }

    this.isSyncing = true

    try {
      logger.info(`[${this.getWorkerName()}] 开始执行定时同步任务`)
      this.sendEvent('sync-started', { timestamp: Date.now() })

      if (!this.syncService) {
        throw new Error('同步服务未初始化')
      }

      const result: SyncResult = await this.syncService.sync()

      this.lastSyncTime = Date.now()

      if (result.success) {
        logger.info(`[${this.getWorkerName()}] 同步任务完成:`, result.message)
        this.sendEvent('sync-completed', {
          timestamp: this.lastSyncTime,
          result: result,
          nextSyncTime: this.nextSyncTime
        })
        this.onOperationSuccess()
      } else {
        logger.warn(`[${this.getWorkerName()}] 同步任务失败:`, result.message)
        this.sendEvent('sync-failed', {
          timestamp: this.lastSyncTime,
          result: result,
          nextSyncTime: this.nextSyncTime
        })
        this.handleCheckError(new Error(result.message), '定时同步')
      }
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 同步任务执行异常:`, error)
      this.sendEvent('sync-error', {
        timestamp: Date.now(),
        error: error instanceof Error ? error.message : '未知错误'
      })
      this.handleCheckError(error, '定时同步')
    } finally {
      this.isSyncing = false

      // 只有在仍然运行状态下才安排下一次同步
      if (this.isRunning) {
        this.scheduleNextSync()
      }
    }
  }

  /**
   * 手动执行同步
   */
  public async manualSync(): Promise<SyncResult> {
    try {
      logger.info(`[${this.getWorkerName()}] 开始执行手动同步`)
      this.sendEvent('manual-sync-started', { timestamp: Date.now() })

      if (!this.syncService) {
        throw new Error('同步服务未初始化，请先启用同步功能或使用带配置的手动同步')
      }

      const result: SyncResult = await this.syncService.sync()

      this.lastSyncTime = Date.now()

      if (result.success) {
        logger.info(`[${this.getWorkerName()}] 手动同步完成:`, result.message)
        this.sendEvent('manual-sync-completed', {
          timestamp: this.lastSyncTime,
          result: result
        })
        this.onOperationSuccess()
      } else {
        logger.warn(`[${this.getWorkerName()}] 手动同步失败:`, result.message)
        this.sendEvent('manual-sync-failed', {
          timestamp: this.lastSyncTime,
          result: result
        })
      }

      return result
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 手动同步执行异常:`, error)
      const errorResult: SyncResult = {
        success: false,
        message: error instanceof Error ? error.message : '未知错误',
        errors: [error instanceof Error ? error.message : '未知错误']
      }

      this.sendEvent('manual-sync-error', {
        timestamp: Date.now(),
        error: errorResult.message
      })

      return errorResult
    }
  }

  /**
   * 使用指定配置执行手动同步
   */
  public async manualSyncWithConfig(config: ScheduledSyncConfig): Promise<SyncResult> {
    try {
      logger.info(`[${this.getWorkerName()}] 开始使用指定配置执行手动同步`)
      this.sendEvent('manual-sync-started', { timestamp: Date.now() })

      // 创建临时的同步服务实例进行同步
      const tempSyncService = this.createSyncService(config.userDataPath)

      // 初始化临时同步服务
      const initialized = await tempSyncService.initialize(
        config.providerType,
        config.providerConfig
      )

      if (!initialized) {
        logger.error(`[${this.getWorkerName()}] 临时同步服务初始化失败`)
        const errorResult: SyncResult = {
          success: false,
          message: '同步服务初始化失败',
          errors: ['同步服务初始化失败']
        }
        this.sendEvent('manual-sync-failed', {
          timestamp: Date.now(),
          result: errorResult
        })
        return errorResult
      }

      // 执行同步
      const result: SyncResult = await tempSyncService.sync()

      // 销毁临时同步服务
      tempSyncService.destroy()

      this.lastSyncTime = Date.now()

      if (result.success) {
        logger.info(`[${this.getWorkerName()}] 使用指定配置手动同步完成:`, result.message)
        this.sendEvent('manual-sync-completed', {
          timestamp: this.lastSyncTime,
          result: result
        })
        this.onOperationSuccess()
      } else {
        logger.warn(`[${this.getWorkerName()}] 使用指定配置手动同步失败:`, result.message)
        this.sendEvent('manual-sync-failed', {
          timestamp: this.lastSyncTime,
          result: result
        })
      }

      return result
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 使用指定配置手动同步执行异常:`, error)
      const errorResult: SyncResult = {
        success: false,
        message: error instanceof Error ? error.message : '未知错误',
        errors: [error instanceof Error ? error.message : '未知错误']
      }

      this.sendEvent('manual-sync-error', {
        timestamp: Date.now(),
        error: errorResult.message
      })

      return errorResult
    }
  }

  /**
   * 测试同步连接
   */
  public async testConnection(): Promise<boolean> {
    try {
      logger.info(`[${this.getWorkerName()}] 开始测试同步连接`)

      if (!this.syncService) {
        logger.error(`[${this.getWorkerName()}] 同步服务未初始化，无法测试连接`)
        return false
      }

      const result = await this.syncService.testConnection()

      if (result) {
        logger.info(`[${this.getWorkerName()}] 连接测试成功`)
        this.sendEvent('connection-test-success', { timestamp: Date.now() })
        this.onOperationSuccess()
      } else {
        logger.warn(`[${this.getWorkerName()}] 连接测试失败`)
        this.sendEvent('connection-test-failed', { timestamp: Date.now() })
      }

      return result
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 连接测试异常:`, error)
      this.sendEvent('connection-test-error', {
        timestamp: Date.now(),
        error: error instanceof Error ? error.message : '未知错误'
      })
      return false
    }
  }

  /**
   * 使用指定配置测试同步连接
   */
  public async testConnectionWithConfig(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      logger.info(
        `[${this.getWorkerName()}] 开始使用指定配置测试同步连接，配置:${JSON.stringify(config)}`
      )

      // 创建临时的同步服务实例进行测试
      const tempSyncService = this.createSyncService(config.userDataPath)

      // 初始化临时同步服务
      const initialized = await tempSyncService.initialize(
        config.providerType,
        config.providerConfig
      )

      if (!initialized) {
        logger.error(`[${this.getWorkerName()}] 临时同步服务初始化失败`)
        this.sendEvent('connection-test-failed', { timestamp: Date.now() })
        return false
      }

      // 测试连接
      const result = await tempSyncService.testConnection()

      // 销毁临时同步服务
      tempSyncService.destroy()

      if (result) {
        logger.info(`[${this.getWorkerName()}] 使用指定配置连接测试成功`)
        this.sendEvent('connection-test-success', { timestamp: Date.now() })
        this.onOperationSuccess()
      } else {
        logger.warn(`[${this.getWorkerName()}] 使用指定配置连接测试失败`)
        this.sendEvent('connection-test-failed', { timestamp: Date.now() })
      }

      return result
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 使用指定配置连接测试异常:`, error)
      this.sendEvent('connection-test-error', {
        timestamp: Date.now(),
        error: error instanceof Error ? error.message : '未知错误'
      })
      return false
    }
  }

  /**
   * 获取同步状态
   */
  public getSyncStatus(): {
    isRunning: boolean
    lastSyncTime: number
    nextSyncTime: number
    syncStatus: SyncStatus
    config: ScheduledSyncConfig | null
  } {
    return {
      isRunning: this.isRunning,
      lastSyncTime: this.lastSyncTime,
      nextSyncTime: this.nextSyncTime,
      syncStatus: this.syncService?.getSyncStatus() || SyncStatus.IDLE,
      config: this.config
    }
  }

  /**
   * 更新配置
   */
  public async updateConfig(newConfig: ScheduledSyncConfig): Promise<boolean> {
    try {
      // 如果正在运行，先停止
      if (this.isRunning) {
        this.stopScheduledSync()
      }

      // 销毁现有的同步服务
      if (this.syncService) {
        this.syncService.destroy()
      }

      // 重新初始化
      const initialized = await this.initialize(newConfig)

      if (initialized && newConfig.enabled) {
        // 如果配置启用且初始化成功，启动定时同步
        return await this.startScheduledSync()
      }

      return initialized
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 更新配置失败:`, error)
      return false
    }
  }

  /**
   * 销毁 Worker
   */
  public destroy(): void {
    try {
      this.stopScheduledSync()
      if (this.syncService) {
        this.syncService.destroy()
      }
      logger.info(`[${this.getWorkerName()}] Worker 已销毁`)
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 销毁 Worker 失败:`, error)
    }
  }

  public handleMessage(message: WorkerMessage): void {
    if (message.type === 'request') {
      this.handleRequest(message as WorkerRequest)
    }
  }

  private async handleRequest(request: WorkerRequest): Promise<void> {
    try {
      let result: unknown

      switch (request.method) {
        case 'initialize':
          result = await this.initialize(
            request.params as Parameters<ScheduledSyncWorker['initialize']>[0]
          )
          break
        case 'startScheduledSync':
          result = await this.startScheduledSync()
          break
        case 'stopScheduledSync':
          result = this.stopScheduledSync()
          break
        case 'manualSync':
          result = await this.manualSync()
          break
        case 'manualSyncWithConfig':
          result = await this.manualSyncWithConfig(
            request.params as Parameters<ScheduledSyncWorker['manualSyncWithConfig']>[0]
          )
          break
        case 'testConnection':
          result = await this.testConnection()
          break
        case 'testConnectionWithConfig':
          result = await this.testConnectionWithConfig(
            request.params as Parameters<ScheduledSyncWorker['testConnectionWithConfig']>[0]
          )
          break
        case 'getSyncStatus':
          result = this.getSyncStatus()
          break
        case 'updateConfig':
          result = await this.updateConfig(
            request.params as Parameters<ScheduledSyncWorker['updateConfig']>[0]
          )
          break
        case 'destroy':
          result = this.destroy()
          break
        case 'exportHeapdump':
          result = this.exportHeapdump(
            request.params as Parameters<ScheduledSyncWorker['exportHeapdump']>[0]
          )
          break
        default:
          throw new Error(`未知方法: ${request.method}`)
      }

      const response: WorkerResponse = {
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: true,
        data: result,
        timestamp: Date.now()
      }
      parentPort?.postMessage(response)
    } catch (error) {
      const response: WorkerResponse = {
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        timestamp: Date.now()
      }
      parentPort?.postMessage(response)
    }
  }
}

const worker = new ScheduledSyncWorker()

parentPort?.on('message', (message: WorkerMessage) => {
  worker.handleMessage(message)
})

// 发送 worker-ready 消息（统一发送时机）
const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: { workerName: '定时同步 Worker' },
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
