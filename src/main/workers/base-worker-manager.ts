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

/**
 * Worker 管理器基类
 * 提供与 Worker Thread 通信的基础功能
 */
export abstract class BaseWorkerManager extends EventEmitter implements WorkerManager {
  protected worker: Worker | null = null
  protected config: WorkerConfig
  protected isInitialized: boolean = false
  protected pendingRequests: Map<string, {
    resolve: (value: any) => void
    reject: (error: Error) => void
    timeout: NodeJS.Timeout
  }> = new Map()
  
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
      console.warn(`[${this.config.name}] Worker 已经在运行中`)
      return
    }

    try {
      console.log(`[${this.config.name}] 启动 Worker:`, this.config.workerPath)
      
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
      console.log(`[${this.config.name}] Worker 启动成功`)
    } catch (error) {
      console.error(`[${this.config.name}] Worker 启动失败:`, error)
      throw error
    }
  }

  /**
   * 停止 Worker
   */
  public async stop(): Promise<void> {
    if (!this.worker) {
      return
    }

    try {
      console.log(`[${this.config.name}] 停止 Worker`)
      
      // 清理所有待处理的请求
      this.clearPendingRequests()
      
      // 终止 Worker
      await this.worker.terminate()
      this.worker = null
      this.isInitialized = false
      
      console.log(`[${this.config.name}] Worker 已停止`)
    } catch (error) {
      console.error(`[${this.config.name}] 停止 Worker 失败:`, error)
      throw error
    }
  }

  /**
   * 调用 Worker 方法
   */
  public async call<T = any>(method: string, params?: any): Promise<T> {
    if (!this.worker || !this.isInitialized) {
      throw new Error(`Worker ${this.config.name} 未初始化`)
    }

    return new Promise((resolve, reject) => {
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
      }, this.requestTimeout)

      // 保存请求信息
      this.pendingRequests.set(requestId, { resolve, reject, timeout })

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
          console.warn(`[${this.config.name}] 未知消息类型:`, JSON.stringify(message))
      }
    } catch (error) {
      console.error(`[${this.config.name}] 处理 Worker 消息失败:`, error)
    }
  }

  /**
   * 处理来自 Worker 的请求（子类可以重写）
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    console.warn(`[${this.config.name}] 未处理的 Worker 请求:`, request.method)
  }

  /**
   * 处理 Worker 响应
   */
  private handleWorkerResponse(response: WorkerResponse): void {
    const pending = this.pendingRequests.get(response.requestId)
    if (!pending) {
      console.warn(`[${this.config.name}] 未找到对应的请求:`, response.requestId)
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
    console.error(`[${this.config.name}] Worker 错误:`, error)
    this.emit('worker-error', { error: error.message })
  }

  /**
   * 处理 Worker 退出
   */
  private handleWorkerExit(code: number): void {
    console.log(`[${this.config.name}] Worker 退出，代码:`, code)
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
