import { parentPort } from 'node:worker_threads'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type { CleanupConfig, CleanupStats } from '../types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板历史记录清理器 Worker
 * 在独立线程中运行，负责定期清理过期的历史记录
 */
class ClipboardHistoryCleanerWorker {
  private config: CleanupConfig
  private cleanupTimer: NodeJS.Timeout | null = null
  private isRunning: boolean = false
  // private memoryDataSyncCallback: ((deletedIds: string[]) => void) | null = null

  constructor() {
    // 默认配置
    this.config = {
      maxKeepTime: -1,
      cleanupInterval: 60 * 60 * 1000, // 默认每小时检查一次
      enableAutoCleanup: false
    }


  }

  /**
   * 初始化清理器
   */
  public initialize(): void {
    // 这里可以添加初始化逻辑，比如从主线程获取配置
  }

  /**
   * 更新配置
   */
  public updateConfig(newConfig: { maxKeepTime: number }): void {
    const oldConfig = { ...this.config }
    
    this.config.maxKeepTime = newConfig.maxKeepTime
    this.config.enableAutoCleanup = newConfig.maxKeepTime !== -1



    // 如果配置发生变化，重新启动自动清理
    if (oldConfig.maxKeepTime !== this.config.maxKeepTime) {
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

    this.isRunning = true
    const startTime = Date.now()

    try {
      // 通过主线程调用数据库操作
      const totalRecords = await this.callMainThread('database-getRecordCount')

      if (totalRecords === 0) {
        return {
          totalRecords: 0,
          expiredRecords: 0,
          deletedRecords: 0,
          cleanupTime: Date.now() - startTime
        }
      }

      // 计算过期时间戳
      const expireTimestamp = this.calculateExpireTimestamp()
      
      // 获取过期记录ID列表
      const expiredRecordIds = await this.callMainThread('database-getExpiredRecordIds', { expireTimestamp })
      
      if (expiredRecordIds.length === 0) {
        return {
          totalRecords,
          expiredRecords: 0,
          deletedRecords: 0,
          cleanupTime: Date.now() - startTime
        }
      }

      // 执行删除操作
      const deletedCount = await this.callMainThread('database-deleteExpiredRecords', { expireTimestamp })

      const cleanupTime = Date.now() - startTime
      const stats: CleanupStats = {
        totalRecords,
        expiredRecords: expiredRecordIds.length,
        deletedRecords: deletedCount,
        cleanupTime
      }

      if (deletedCount > 0) {
        // 通知内存数据同步
        this.sendEvent('memory-sync-required', { deletedIds: expiredRecordIds })

        // 通知清理完成
        this.sendEvent('cleanup-completed', { stats })
      }

      return stats
    } catch (error) {
      log.error('[历史记录清理器 Worker] 清理操作失败:', error)
      this.sendEvent('cleanup-error', { error: error instanceof Error ? error.message : String(error) })
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
   * 设置内存数据同步回调
   */
  public setMemoryDataSyncCallback(): void {
    // 在 Worker 中，这个方法主要是为了兼容原有接口
    // 实际的同步通过事件机制实现
  }

  /**
   * 同步删除的记录（由主线程调用）
   */
  public syncDeletedRecords(_: string[]): void {
    // 这个方法在 Worker 中不需要实现，因为同步是通过事件机制处理的
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
   * 调用主线程方法
   */
  private async callMainThread(method: string, params?: any): Promise<any> {
    return new Promise((resolve, reject) => {
      const requestId = `worker-${Date.now()}-${Math.random()}`
      
      const request: WorkerRequest = {
        id: requestId,
        type: 'request',
        method,
        params,
        timestamp: Date.now()
      }

      // 设置超时
      const timeout = setTimeout(() => {
        reject(new Error(`调用主线程方法超时: ${method}`))
      }, 30000)

      // 监听响应
      const messageHandler = (message: WorkerMessage) => {
        if (message.type === 'response' && (message as WorkerResponse).requestId === requestId) {
          clearTimeout(timeout)
          parentPort?.off('message', messageHandler)
          
          const response = message as WorkerResponse
          if (response.success) {
            resolve(response.data)
          } else {
            reject(new Error(response.error || '主线程调用失败'))
          }
        }
      }

      parentPort?.on('message', messageHandler)
      parentPort?.postMessage(request)
    })
  }

  /**
   * 发送事件到主线程
   */
  private sendEvent(event: string, data?: any): void {
    const eventMessage: WorkerEvent = {
      id: `event-${Date.now()}-${Math.random()}`,
      type: 'event',
      event,
      data,
      timestamp: Date.now()
    }
    parentPort?.postMessage(eventMessage)
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
          result = this.initialize()
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
        case 'setMemoryDataSyncCallback':
          result = this.setMemoryDataSyncCallback()
          break
        case 'syncDeletedRecords':
          result = this.syncDeletedRecords(request.params)
          break
        case 'destroy':
          result = this.destroy()
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

// Worker 准备就绪，发送 ready 事件
log.info('[历史记录清理器 Worker] Worker 已准备就绪，等待初始化请求')

// 发送 worker-ready 事件，告知主线程 Worker 已启动
const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: {},
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
