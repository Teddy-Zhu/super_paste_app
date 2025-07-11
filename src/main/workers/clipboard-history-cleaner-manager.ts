import { BaseWorkerManager } from './base-worker-manager'
import databaseService from '../services/database-service'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import type { CleanupConfig, CleanupStats } from '../types/clipboard-types'

/**
 * 剪贴板历史记录清理器 Worker 管理器
 * 负责管理清理器 Worker 的生命周期和通信
 */
export class ClipboardHistoryCleanerManager extends BaseWorkerManager {
  private memoryDataSyncCallback: ((deletedIds: string[]) => void) | null = null

  constructor(workerPath: string) {
    const config: WorkerConfig = {
      name: 'ClipboardHistoryCleaner',
      workerPath
    }
    super(config)

    // 监听 Worker 事件
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
    this.on('cleanup-completed', (data: { stats: CleanupStats }) => {
      console.log('[清理器管理器] 清理完成:', data.stats)
    })

    // 监听清理错误事件
    this.on('cleanup-error', (data: { error: string }) => {
      console.error('[清理器管理器] 清理错误:', data.error)
    })
  }

  /**
   * 启动 Worker 并设置数据库调用处理
   */
  public async start(): Promise<void> {
    await super.start()
    
    // 设置数据库调用处理
    this.setupDatabaseCallHandlers()
  }

  /**
   * 设置数据库调用处理器
   * Worker 通过这些处理器调用主线程的数据库操作
   */
  private setupDatabaseCallHandlers(): void {
    // 不需要额外的消息监听器，使用基类的 handleWorkerRequest 方法
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

  // ==================== 数据库操作方法已移至 DatabaseService ====================

  // ==================== 公共 API 方法 ====================

  /**
   * 初始化清理器
   */
  public async initialize(): Promise<void> {
    return this.call('initialize')
  }

  /**
   * 更新配置
   */
  public async updateConfig(config: { maxKeepTime: number }): Promise<void> {
    return this.call('updateConfig', config)
  }

  /**
   * 获取配置
   */
  public async getConfig(): Promise<CleanupConfig> {
    return this.call('getConfig')
  }

  /**
   * 启动自动清理
   */
  public async startAutoCleanup(): Promise<void> {
    return this.call('startAutoCleanup')
  }

  /**
   * 停止自动清理
   */
  public async stopAutoCleanup(): Promise<void> {
    return this.call('stopAutoCleanup')
  }

  /**
   * 执行清理操作
   */
  public async performCleanup(): Promise<CleanupStats> {
    return this.call('performCleanup')
  }

  /**
   * 设置内存数据同步回调
   */
  public setMemoryDataSyncCallback(callback: (deletedIds: string[]) => void): void {
    this.memoryDataSyncCallback = callback
    // 通知 Worker 回调已设置
    this.call('setMemoryDataSyncCallback').catch(error => {
      console.error('[清理器管理器] 设置内存同步回调失败:', error)
    })
  }

  /**
   * 同步删除的记录
   */
  public async syncDeletedRecords(deletedIds: string[]): Promise<void> {
    return this.call('syncDeletedRecords', deletedIds)
  }

  /**
   * 销毁清理器
   */
  public async destroy(): Promise<void> {
    try {
      await this.call('destroy')
    } catch (error) {
      console.error('[清理器管理器] 销毁清理器失败:', error)
    } finally {
      await this.stop()
    }
  }
}
