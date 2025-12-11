import { parentPort } from 'node:worker_threads'
import { writeHeapSnapshot } from 'v8'
import { join } from 'path'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import logger from '../utils/logger'

/**
 * Worker 初始化配置接口
 */
export interface WorkerInitConfig {
  /** 用户数据目录路径 */
  userDataPath: string
  /** 日志文件名（可选，默认使用 worker 名称） */
  logFileName?: string
  /** 日志作用域（可选，默认使用 worker 名称） */
  logScope?: string
  /** 其他配置参数 */
  [key: string]: any
}

/**
 * Worker 基类
 * 提供 Worker 线程内部通信的基础功能，包括重试机制和错误处理
 * 统一处理日志初始化和基础功能
 */
export abstract class BaseWorker {
  protected consecutiveErrors: number = 0
  protected maxConsecutiveErrors: number = 5
  protected errorBackoffTime: number = 0
  protected isLoggerInitialized: boolean = false

  /**
   * 统一初始化 Worker
   * 子类应该调用此方法来初始化基础功能
   */
  protected initializeWorker(config: WorkerInitConfig): void {
    // 初始化日志系统
    this.initializeLogger(config)

    // 子类可以重写此方法来添加额外的初始化逻辑
    this.onWorkerInitialized(config)
  }

  /**
   * 初始化 Worker 日志系统
   */
  private initializeLogger(config: WorkerInitConfig): void {
    if (this.isLoggerInitialized) {
      return
    }

    const workerName = this.getWorkerName()
    const fileName =
      config.logFileName || `${workerName.toLowerCase().replace(/\s+/g, '-')}-worker.log`
    const scope = config.logScope || workerName.toLowerCase().replace(/\s+/g, '-')

    logger.initialize({
      userDataPath: config.userDataPath,
      fileName,
      scope
    })

    this.isLoggerInitialized = true
    logger.info(`[${workerName}] Worker 日志系统初始化完成`)
  }

  /**
   * 发送 Worker 准备就绪事件
   * 子类可以在适当的时机调用此方法
   */
  protected sendWorkerReadyEvent(): void {
    const readyEvent: WorkerEvent = {
      id: `event-${Date.now()}-${Math.random()}`,
      type: 'event',
      event: 'worker-ready',
      data: { workerName: this.getWorkerName() },
      timestamp: Date.now()
    }
    logger.info(`[${this.getWorkerName()}] Worker 准备就绪，等待初始化请求`)
    parentPort?.postMessage(readyEvent)
  }

  /**
   * Worker 初始化完成后的回调（子类可以重写）
   */
  protected onWorkerInitialized(_config: WorkerInitConfig): void {
    // 子类可以重写此方法来添加额外的初始化逻辑
  }

  /**
   * 发送事件到主线程
   */
  protected sendEvent(event: string, data?: any): void {
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
   * 睡眠指定毫秒数
   */
  protected sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * 处理检查错误（可被子类重写）
   */
  protected handleCheckError(error: any, operation: string = '操作'): void {
    this.consecutiveErrors++

    const isTimeoutError = error instanceof Error && error.message.includes('超时')

    if (isTimeoutError) {
      logger.warn(
        `[${this.getWorkerName()}] ${operation}超时 (连续错误: ${this.consecutiveErrors}/${this.maxConsecutiveErrors})`
      )

      // 如果连续超时错误过多，启用退避机制
      if (this.consecutiveErrors >= this.maxConsecutiveErrors) {
        const backoffDuration = Math.min(30000, 5000 * this.consecutiveErrors) // 最多30秒
        this.errorBackoffTime = Date.now() + backoffDuration

        logger.warn(
          `[${this.getWorkerName()}] 连续超时错误过多，暂停${operation} ${backoffDuration}ms`
        )
      }
    } else {
      logger.error(`[${this.getWorkerName()}] ${operation}失败:`, error)
    }
  }

  /**
   * 重置错误状态
   */
  public resetErrorState(): void {
    this.consecutiveErrors = 0
    this.errorBackoffTime = 0
    logger.info(`[${this.getWorkerName()}] 错误状态已重置`)
  }

  /**
   * 导出 Worker 的内存快照
   */
  public exportHeapdump(desktopPath: string): string {
    try {
      const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
      const workerName = this.getWorkerName().replace(/\s+/g, '-').toLowerCase()
      const filename = `heapdump-worker-${workerName}-${timestamp}.heapsnapshot`
      const filepath = join(desktopPath, filename)

      // 使用 V8 内置的 writeHeapSnapshot 导出内存快照
      const actualFilepath = writeHeapSnapshot(filepath)

      logger.info(`[${this.getWorkerName()}] Worker heapdump 导出成功:`, actualFilepath)

      return actualFilepath
    } catch (error) {
      logger.error(`[${this.getWorkerName()}] 导出 Worker heapdump 失败:`, error)
      throw error
    }
  }

  /**
   * 检查是否在错误退避期间
   */
  protected isInErrorBackoff(): boolean {
    return this.errorBackoffTime > Date.now()
  }

  /**
   * 成功操作后重置错误计数
   */
  protected onOperationSuccess(): void {
    this.consecutiveErrors = 0
  }

  /**
   * 获取Worker名称（子类需要实现）
   */
  protected abstract getWorkerName(): string

  /**
   * 处理来自主线程的消息（子类需要实现）
   */
  public abstract handleMessage(message: WorkerMessage): void
}
