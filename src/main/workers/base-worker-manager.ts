import { Worker } from 'node:worker_threads'
import { EventEmitter } from 'node:events'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent,
  WorkerManager,
  WorkerConfig
} from '../types/worker-types'
import logger from '../utils/logger'

/**
 * Worker 管理器基类
 * 提供与 Worker Thread 通信的基础功能
 */
export abstract class BaseWorkerManager extends EventEmitter implements WorkerManager {
  protected worker: Worker | null = null
  protected config: WorkerConfig
  protected isInitialized: boolean = false
  protected pendingRequests: Map<
    string,
    {
      resolve: (value: unknown) => void
      reject: (error: Error) => void
      timeout: NodeJS.Timeout
    }
  > = new Map()

  private requestTimeout: number = 30000 // 30秒超时
  private messageIdCounter: number = 0

  constructor(config: WorkerConfig) {
    super()
    this.config = config
  }

  /**
   * 启动 Worker
   */
  public async start(): Promise<void> {
    if (this.worker) {
      return
    }

    try {
      this.worker = new Worker(this.config.workerPath, this.config.options)

      // 设置消息监听
      this.worker.on('message', this.handleWorkerMessage.bind(this))

      // 设置错误监听
      this.worker.on('error', this.handleWorkerError.bind(this))

      // 设置退出监听
      this.worker.on('exit', this.handleWorkerExit.bind(this))

      // 等待 Worker 初始化完成
      await this.waitForWorkerReady()

      this.isInitialized = true
    } catch (error) {
      logger.error(`[${this.config.name}] Worker 启动失败:`, error)
      throw error
    }
  }

  /**
   * 停止 Worker
   */
  public async stop(): Promise<void> {
    if (!this.worker) {
      logger.debug(`[${this.config.name}] Worker 不存在,跳过停止`)
      return
    }

    logger.info(`[${this.config.name}] 开始停止 Worker...`)

    try {
      // 清理所有待处理的请求
      logger.debug(`[${this.config.name}] 清理待处理的请求 (${this.pendingRequests.size} 个)...`)
      this.clearPendingRequests()
      logger.debug(`[${this.config.name}] 待处理的请求已清理`)

      // 给 Worker 一点时间完成清理(如果有destroy方法的话)
      logger.debug(`[${this.config.name}] 等待 100ms 让 Worker 完成清理...`)
      await new Promise((resolve) => setTimeout(resolve, 100))

      // 设置终止超时,避免无限等待
      logger.debug(`[${this.config.name}] 开始终止 Worker (5秒超时)...`)
      const terminatePromise = this.worker.terminate()
      let terminationTimeout: NodeJS.Timeout
      const timeoutPromise = new Promise<never>((_, reject) => {
        terminationTimeout = setTimeout(() => reject(new Error('Worker 终止超时')), 5000)
      })

      try {
        await Promise.race([terminatePromise, timeoutPromise])
      } finally {
        clearTimeout(terminationTimeout!)
      }

      this.worker = null
      this.isInitialized = false

      logger.info(`[${this.config.name}] ✓ Worker 已成功停止`)
    } catch (error) {
      logger.error(`[${this.config.name}] ✗ 停止 Worker 失败:`, error)
      // 即使失败也要清理引用
      this.worker = null
      this.isInitialized = false
      throw error
    }
  }

  /**
   * 调用 Worker 方法
   */
  public async call<T = unknown>(
    method: string,
    params?: unknown,
    timeoutMs = this.requestTimeout
  ): Promise<T> {
    if (!this.worker || !this.isInitialized) {
      throw new Error(`Worker ${this.config.name} 未初始化`)
    }

    return new Promise<T>((resolve, reject) => {
      const requestId = this.generateRequestId()

      const request: WorkerRequest = {
        id: requestId,
        type: 'request',
        method,
        params,
        timestamp: Date.now()
      }

      // 设置超时
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(requestId)
        reject(new Error(`Worker 方法调用超时: ${method}`))
      }, timeoutMs)

      // 保存请求信息
      this.pendingRequests.set(requestId, {
        resolve: (value) => resolve(value as T),
        reject,
        timeout
      })

      // 发送请求
      this.worker!.postMessage(request)
    })
  }

  /**
   * 检查 Worker 是否运行中
   */
  public isRunning(): boolean {
    return this.worker !== null && this.isInitialized
  }

  /**
   * 导出 Worker 的内存快照
   */
  public async exportWorkerHeapdump(desktopPath: string): Promise<string | null> {
    if (!this.isRunning()) {
      logger.warn(`[${this.config.name}] Worker 未运行，跳过 heapdump 导出`)
      return null
    }

    try {
      const result = await this.call<string>('exportHeapdump', desktopPath)
      logger.info(`[${this.config.name}] Worker heapdump 导出成功:`, result)
      return result
    } catch (error) {
      logger.error(`[${this.config.name}] Worker heapdump 导出失败:`, error)
      return null
    }
  }

  /**
   * 处理 Worker 消息
   */
  private handleWorkerMessage(message: WorkerMessage): void {
    try {
      switch (message.type) {
        case 'response':
          this.handleWorkerResponse(message as WorkerResponse)
          break
        case 'event':
          this.handleWorkerEvent(message as WorkerEvent)
          break
        case 'request':
          // 允许子类处理来自 Worker 的请求
          this.handleWorkerRequest(message as WorkerRequest)
          break
        default:
          logger.warn(`[${this.config.name}] 未知消息类型:`, JSON.stringify(message))
      }
    } catch (error) {
      logger.error(`[${this.config.name}] 处理 Worker 消息失败:`, error)
    }
  }

  /**
   * 处理来自 Worker 的请求（子类可以重写）
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    logger.warn(`[${this.config.name}] 未处理的 Worker 请求:`, request.method)
  }

  /**
   * 处理 Worker 响应
   */
  private handleWorkerResponse(response: WorkerResponse): void {
    const pending = this.pendingRequests.get(response.requestId)
    if (!pending) {
      logger.warn(`[${this.config.name}] 未找到对应的请求:`, response.requestId)
      return
    }

    // 清理超时定时器
    clearTimeout(pending.timeout)
    this.pendingRequests.delete(response.requestId)

    // 处理响应
    if (response.success) {
      pending.resolve(response.data)
    } else {
      pending.reject(new Error(response.error || 'Worker 调用失败'))
    }
  }

  /**
   * 处理 Worker 事件
   */
  private handleWorkerEvent(event: WorkerEvent): void {
    this.emit(event.event, event.data)
  }

  /**
   * 处理 Worker 错误
   */
  private handleWorkerError(error: Error): void {
    logger.error(`[${this.config.name}] Worker 错误:`, error)
    this.emit('worker-error', { error: error.message })
  }

  /**
   * 处理 Worker 退出
   */
  private handleWorkerExit(code: number): void {
    this.isInitialized = false
    this.worker = null
    this.clearPendingRequests()
    this.emit('worker-exit', { code })
  }

  /**
   * 等待 Worker 准备就绪
   */
  private async waitForWorkerReady(): Promise<void> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.off('worker-ready', onReady)
        reject(new Error(`Worker ${this.config.name} 初始化超时`))
      }, 10000) // 10秒超时

      const onReady = (): void => {
        clearTimeout(timeout)
        this.off('worker-ready', onReady)
        resolve()
      }

      this.on('worker-ready', onReady)
    })
  }

  /**
   * 生成请求 ID
   */
  private generateRequestId(): string {
    return `${this.config.name}-${Date.now()}-${++this.messageIdCounter}`
  }

  /**
   * 清理所有待处理的请求
   */
  private clearPendingRequests(): void {
    for (const [, pending] of this.pendingRequests) {
      clearTimeout(pending.timeout)
      pending.reject(new Error('Worker 已停止'))
    }
    this.pendingRequests.clear()
  }
}
