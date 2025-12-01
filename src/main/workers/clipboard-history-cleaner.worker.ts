import { parentPort } from 'node:worker_threads'
import { BaseWorker } from './base-worker'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type { CleanupConfig, CleanupStats } from '../types/clipboard-types'
import logger from '../utils/logger'

/**
 * 剪贴板历史记录清理器 Worker
 * 在独立线程中运行，负责定期清理过期的历史记录
 */
class ClipboardHistoryCleanerWorker extends BaseWorker {
  private config: CleanupConfig
  private cleanupTimer: NodeJS.Timeout | null = null
  private isRunning: boolean = false

  constructor() {
    super()

    // 默认配置
    this.config = {
      maxKeepTime: -1,
      maxHistorySize: -1,
      cleanupInterval: 10 * 60 * 1000, // 默认每小时检查一次
      enableAutoCleanup: false,
      enableCountBasedCleanup: false
    }
  }

  /**
   * 获取Worker名称
   */
  protected getWorkerName(): string {
    return '剪贴板历史清理器 Worker'
  }

  /**
   * 初始化清理器
   */
  public initialize(
    params?: { config?: Partial<CleanupConfig>; userDataPath: string } | string
  ): void {
    // 处理参数兼容性
    let config: Partial<CleanupConfig> | undefined
    let userDataPath: string | undefined

    if (typeof params === 'string') {
      // 旧格式：直接传 userDataPath 字符串
      userDataPath = params
    } else if (params && typeof params === 'object') {
      // 新格式：{ config, userDataPath }
      config = params.config
      userDataPath = params.userDataPath
    } else {
      userDataPath = ''
    }

    // 初始化 Worker 基础功能（包括日志系统）
    this.initializeWorker({
      userDataPath,
      logFileName: 'clipboard-cleaner-worker.log',
      logScope: 'clipboard-cleaner-worker'
    })
    // 更新配置
    if (config) {
      this.config = { ...this.config, ...config }

      // 如果启用了自动清理，立即启动
      if (this.config.enableAutoCleanup) {
        this.startAutoCleanup()
      }
    }
  }

  /**
   * 更新配置
   */
  public updateConfig(newConfig: Partial<CleanupConfig>): void {
    const oldConfig = { ...this.config }

    // 更新所有提供的配置项
    this.config = { ...this.config, ...newConfig }

    // 自动设置 enableAutoCleanup 基于 maxKeepTime
    if ('maxKeepTime' in newConfig) {
      this.config.enableAutoCleanup = this.config.maxKeepTime !== -1
    }

    // 如果关键配置发生变化，重新启动自动清理
    const shouldRestart =
      oldConfig.maxKeepTime !== this.config.maxKeepTime ||
      oldConfig.enableAutoCleanup !== this.config.enableAutoCleanup ||
      oldConfig.cleanupInterval !== this.config.cleanupInterval

    if (shouldRestart) {
      this.stopAutoCleanup()
      if (this.config.enableAutoCleanup) {
        this.startAutoCleanup()
      }
    }
  }

  /**
   * 获取当前配置
   */
  public getConfig(): CleanupConfig {
    return { ...this.config }
  }

  /**
   * 启动自动清理
   */
  public startAutoCleanup(): void {
    if (!this.config.enableAutoCleanup) {
      return
    }

    if (this.cleanupTimer) {
      return
    }

    // 立即执行一次清理
    this.performCleanup()

    // 设置定时清理
    this.cleanupTimer = setInterval(() => {
      this.performCleanup()
    }, this.config.cleanupInterval)
  }

  /**
   * 停止自动清理
   */
  public stopAutoCleanup(): void {
    if (this.cleanupTimer) {
      clearInterval(this.cleanupTimer)
      this.cleanupTimer = null
    }
  }

  /**
   * 执行清理操作
   */
  public async performCleanup(): Promise<CleanupStats> {
    if (this.isRunning) {
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }

    if (!this.config.enableAutoCleanup) {
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }

    // 如果在错误退避期间，跳过清理
    if (this.isInErrorBackoff()) {
      logger.info('[历史记录清理器 Worker] 在错误退避期间，跳过清理操作')
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }

    this.isRunning = true
    const startTime = Date.now()

    try {
      // 通过主线程调用数据库操作
      const totalRecords = await this.callMainThread('database-getRecordCount')

      if (totalRecords === 0) {
        this.onOperationSuccess()
        return {
          totalRecords: 0,
          expiredRecords: 0,
          deletedRecords: 0,
          cleanupTime: Date.now() - startTime
        }
      }

      let deletedRecordIds: string[] = []
      let deletedCount = 0

      // 1. 基于时间的清理
      if (this.config.maxKeepTime > 0) {
        const expireTimestamp = this.calculateExpireTimestamp()
        const expiredRecordIds = await this.callMainThread('database-getExpiredRecordIds', {
          expireTimestamp
        })

        if (expiredRecordIds.length > 0) {
          // 优化：直接删除，无需查询 hash
          const timeBasedDeletedCount = await this.callMainThread('database-deleteExpiredRecords', {
            expireTimestamp
          })
          deletedRecordIds.push(...expiredRecordIds)
          deletedCount += timeBasedDeletedCount
        }
      }

      // 2. 基于数量的清理（排除有标签的记录）
      if (this.config.enableCountBasedCleanup && this.config.maxHistorySize > 0) {
        const currentCount = await this.callMainThread('database-getRecordCount')

        if (currentCount > this.config.maxHistorySize) {
          const excessCount = currentCount - this.config.maxHistorySize

          // 优化：只获取需要删除的无标签记录 ID，避免删除有标签的记录
          const recordsToDelete = await this.callMainThread(
            'database-queryRecordIdsAndHashesWithoutTags',
            {
              limit: excessCount,
              offset: this.config.maxHistorySize,
              orderBy: 'last_used',
              orderDirection: 'DESC'
            }
          )

          // 删除这些记录（无需解绑标签，因为这些记录没有标签）
          for (const record of recordsToDelete) {
            await this.callMainThread('database-deleteRecord', { contentId: record.content_id })
            deletedRecordIds.push(record.content_id)
            deletedCount++
          }
        }
      }

      if (deletedCount === 0) {
        this.onOperationSuccess()
        return {
          totalRecords,
          expiredRecords: 0,
          deletedRecords: 0,
          cleanupTime: Date.now() - startTime
        }
      }

      const cleanupTime = Date.now() - startTime
      const stats: CleanupStats = {
        totalRecords,
        expiredRecords: deletedRecordIds.length,
        deletedRecords: deletedCount,
        cleanupTime
      }

      if (deletedCount > 0) {
        // 通知清理完成，只传递删除的记录 ID
        this.sendEvent('cleanup-completed', {
          stats,
          deletedRecordIds
        })
      }

      // 成功完成清理，重置错误计数
      this.onOperationSuccess()

      return stats
    } catch (error) {
      this.handleCheckError(error, '历史记录清理')
      this.sendEvent('cleanup-error', {
        error: error instanceof Error ? error.message : String(error)
      })
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: Date.now() - startTime
      }
    } finally {
      this.isRunning = false
    }
  }

  /**
   * 检查是否需要基于数量的清理
   */
  public async checkCountBasedCleanup(maxHistorySize: number): Promise<void> {
    if (!this.config.enableCountBasedCleanup || maxHistorySize <= 0) {
      return
    }

    try {
      const currentCount = await this.callMainThread('database-getRecordCount')

      if (currentCount > maxHistorySize) {
        const excessCount = currentCount - maxHistorySize

        // 优化：只获取需要删除的无标签记录 ID，避免删除有标签的记录
        const recordsToDelete = await this.callMainThread(
          'database-queryRecordIdsAndHashesWithoutTags',
          {
            limit: excessCount,
            offset: maxHistorySize,
            orderBy: 'last_used',
            orderDirection: 'DESC'
          }
        )

        const deletedIds: string[] = []

        // 删除这些记录（无需解绑标签，因为这些记录没有标签）
        for (const record of recordsToDelete) {
          await this.callMainThread('database-deleteRecord', { contentId: record.content_id })
          deletedIds.push(record.content_id)
        }

        if (deletedIds.length > 0) {
          logger.info(
            `[历史记录清理器 Worker] 基于数量清理完成，删除了 ${deletedIds.length} 条记录`
          )

          // 通知清理完成，只传递删除的记录 ID
          this.sendEvent('cleanup-completed', {
            stats: {
              totalRecords: currentCount,
              expiredRecords: 0,
              deletedRecords: deletedIds.length,
              cleanupTime: 0
            },
            deletedRecordIds: deletedIds
          })
        }
      }
    } catch (error) {
      logger.error('[历史记录清理器 Worker] 基于数量的清理失败:', error)
    }
  }

  /**
   * 销毁清理器
   */
  public destroy(): void {
    this.stopAutoCleanup()

    this.sendEvent('worker-destroyed', {})
  }

  /**
   * 计算过期时间戳
   */
  private calculateExpireTimestamp(): number {
    const now = Date.now()
    const maxKeepTimeMs = this.config.maxKeepTime * 24 * 60 * 60 * 1000 // 转换为毫秒
    return now - maxKeepTimeMs
  }

  /**
   * 处理来自主线程的消息
   */
  public handleMessage(message: WorkerMessage): void {
    if (message.type === 'request') {
      this.handleRequest(message as WorkerRequest)
    }
  }

  /**
   * 处理请求
   */
  private async handleRequest(request: WorkerRequest): Promise<void> {
    try {
      let result: any

      switch (request.method) {
        case 'initialize':
          result = this.initialize(request.params)
          break
        case 'updateConfig':
          result = this.updateConfig(request.params)
          break
        case 'getConfig':
          result = this.getConfig()
          break
        case 'startAutoCleanup':
          result = this.startAutoCleanup()
          break
        case 'stopAutoCleanup':
          result = this.stopAutoCleanup()
          break
        case 'performCleanup':
          result = await this.performCleanup()
          break
        case 'checkCountBasedCleanup':
          result = await this.checkCountBasedCleanup(request.params)
          break
        case 'destroy':
          result = this.destroy()
          break
        case 'exportHeapdump':
          result = this.exportHeapdump(request.params)
          break
        default:
          throw new Error(`未知方法: ${request.method}`)
      }

      // 发送成功响应
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
      // 发送错误响应
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

// 创建 Worker 实例
const worker = new ClipboardHistoryCleanerWorker()

// 监听主线程消息
parentPort?.on('message', (message: WorkerMessage) => {
  worker.handleMessage(message)
})

// 发送 worker-ready 消息（统一发送时机）
const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: { workerName: '历史记录清理器 Worker' },
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
