import { parentPort } from 'node:worker_threads'
import { writeHeapSnapshot } from 'v8'
import { join } from 'path'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import log from 'electron-log'

/**
 * Worker 基类
 * 提供 Worker 线程内部通信的基础功能，包括重试机制和错误处理
 */
export abstract class BaseWorker {
  protected consecutiveErrors: number = 0
  protected maxConsecutiveErrors: number = 5
  protected errorBackoffTime: number = 0

  /**
   * 调用主线程方法（带重试机制）
   */
  protected async callMainThread(method: string, params?: any): Promise<any> {
    const maxRetries = 3
    const baseDelay = 1000 // 1秒基础延迟
    
    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        return await this.callMainThreadOnce(method, params, attempt)
      } catch (error) {
        const isTimeoutError = error instanceof Error && error.message.includes('超时')
        const isLastAttempt = attempt === maxRetries
        
        if (isTimeoutError && !isLastAttempt) {
          // 如果是超时错误且不是最后一次尝试，则等待后重试
          const delay = baseDelay * Math.pow(2, attempt) // 指数退避
          log.warn(`[${this.getWorkerName()}] ${method} 调用超时，${delay}ms后进行第${attempt + 2}次尝试`)
          await this.sleep(delay)
          continue
        }
        
        // 如果不是超时错误或者是最后一次尝试，直接抛出错误
        throw error
      }
    }
    
    throw new Error(`调用主线程方法失败: ${method}`)
  }

  /**
   * 单次调用主线程方法
   */
  private async callMainThreadOnce(method: string, params?: any, attempt: number = 0): Promise<any> {
    return new Promise((resolve, reject) => {
      const requestId = `worker-${Date.now()}-${Math.random()}`

      const request: WorkerRequest = {
        id: requestId,
        type: 'request',
        method,
        params,
        timestamp: Date.now()
      }

      // 根据尝试次数调整超时时间
      const timeoutMs = attempt === 0 ? 30000 : 60000 // 第一次30秒，重试时60秒

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

      const timeout = setTimeout(() => {
        // 修复：超时时也要移除监听器，防止内存泄漏
        parentPort?.off('message', messageHandler)
        reject(new Error(`调用主线程方法超时: ${method}`))
      }, timeoutMs)

      parentPort?.on('message', messageHandler)
      parentPort?.postMessage(request)
    })
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
    return new Promise(resolve => setTimeout(resolve, ms))
  }

  /**
   * 处理检查错误（可被子类重写）
   */
  protected handleCheckError(error: any, operation: string = '操作'): void {
    this.consecutiveErrors++
    
    const isTimeoutError = error instanceof Error && error.message.includes('超时')
    
    if (isTimeoutError) {
      log.warn(`[${this.getWorkerName()}] ${operation}超时 (连续错误: ${this.consecutiveErrors}/${this.maxConsecutiveErrors})`)
      
      // 如果连续超时错误过多，启用退避机制
      if (this.consecutiveErrors >= this.maxConsecutiveErrors) {
        const backoffDuration = Math.min(30000, 5000 * this.consecutiveErrors) // 最多30秒
        this.errorBackoffTime = Date.now() + backoffDuration
        
        log.warn(`[${this.getWorkerName()}] 连续超时错误过多，暂停${operation} ${backoffDuration}ms`)
      }
    } else {
      log.error(`[${this.getWorkerName()}] ${operation}失败:`, error)
    }
  }

  /**
   * 重置错误状态
   */
  public resetErrorState(): void {
    this.consecutiveErrors = 0
    this.errorBackoffTime = 0
    log.info(`[${this.getWorkerName()}] 错误状态已重置`)
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

      log.info(`[${this.getWorkerName()}] Worker heapdump 导出成功:`, actualFilepath)

      return actualFilepath
    } catch (error) {
      log.error(`[${this.getWorkerName()}] 导出 Worker heapdump 失败:`, error)
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
