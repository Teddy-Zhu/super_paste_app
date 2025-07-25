import logger from '../utils/logger'
import {
  SyncStatus,
  SyncProviderType
} from './types'
import type {
  SyncProviderConfig,
  SyncResult,
  SyncEvent,
  SyncEventListener,
  SyncFileInfo
} from './types'

/**
 * 抽象同步提供者基类
 * 定义了所有同步提供者必须实现的接口
 */
export abstract class BaseSyncProvider {
  protected config: SyncProviderConfig
  protected status: SyncStatus = SyncStatus.IDLE
  protected eventListeners: Map<string, SyncEventListener[]> = new Map()
  protected isInitialized = false

  constructor(config: SyncProviderConfig) {
    this.config = config
  }

  abstract name(): SyncProviderType

  /**
   * 初始化同步提供者
   */
  abstract initialize(): Promise<boolean>

  /**
   * 测试连接
   */
  abstract testConnection(): Promise<boolean>

  /**
   * 上传文件
   * @param localPath 本地文件路径
   * @param remotePath 远程文件路径
   */
  abstract uploadFile(localPath: string, remotePath: string): Promise<boolean>

  /**
   * 直接上传文件内容
   * @param fileContent 文件内容（字符串或Buffer）
   * @param remotePath 远程文件路径
   */
  abstract uploadFileWithData(fileContent: string | Buffer, remotePath: string): Promise<boolean>

  /**
   * 下载文件
   * @param remotePath 远程文件路径
   * @param localPath 本地文件路径
   */
  abstract downloadFile(remotePath: string, localPath: string): Promise<boolean>

  /**
   * 删除远程文件
   * @param remotePath 远程文件路径
   */
  abstract deleteFile(remotePath: string): Promise<boolean>

  /**
   * 列出远程文件
   * @param remotePath 远程目录路径
   */
  abstract listFiles(remotePath?: string): Promise<SyncFileInfo[]>

  /**
   * 检查远程文件是否存在
   * @param remotePath 远程文件路径
   */
  abstract fileExists(remotePath: string): Promise<boolean>

  /**
   * 获取远程文件信息
   * @param remotePath 远程文件路径
   */
  abstract getFileInfo(remotePath: string): Promise<SyncFileInfo | null>

  /**
   * 执行完整同步
   */
  async sync(): Promise<SyncResult> {
    if (!this.isInitialized) {
      const initResult = await this.initialize()
      if (!initResult) {
        return {
          success: false,
          message: '同步提供者初始化失败',
          errors: ['初始化失败']
        }
      }
    }

    if (this.status === SyncStatus.SYNCING) {
      return {
        success: false,
        message: '同步正在进行中',
        errors: ['同步正在进行中']
      }
    }

    this.setStatus(SyncStatus.SYNCING)
    
    try {
      const result = await this.performSync()
      this.setStatus(result.success ? SyncStatus.SUCCESS : SyncStatus.ERROR)
      return result
    } catch (error) {
      logger.error('[同步提供者] 同步过程中发生错误:', error)
      this.setStatus(SyncStatus.ERROR)
      return {
        success: false,
        message: `同步失败: ${error instanceof Error ? error.message : '未知错误'}`,
        errors: [error instanceof Error ? error.message : '未知错误']
      }
    }
  }

  /**
   * 执行具体的同步逻辑
   * 子类需要实现此方法
   */
  protected abstract performSync(): Promise<SyncResult>

  /**
   * 获取当前状态
   */
  getStatus(): SyncStatus {
    return this.status
  }

  /**
   * 获取配置
   */
  getConfig(): SyncProviderConfig {
    return { ...this.config }
  }

  /**
   * 更新配置
   */
  updateConfig(newConfig: Partial<SyncProviderConfig>): void {
    this.config = { ...this.config, ...newConfig }
    this.emitEvent('config-updated', this.config)
  }

  /**
   * 设置状态
   */
  protected setStatus(status: SyncStatus): void {
    if (this.status !== status) {
      this.status = status
      this.emitEvent('status-changed', { status, timestamp: new Date() })
      logger.debug(`[同步提供者] 状态变更为: ${status}`)
    }
  }

  /**
   * 添加事件监听器
   */
  on(eventType: string, listener: SyncEventListener): void {
    if (!this.eventListeners.has(eventType)) {
      this.eventListeners.set(eventType, [])
    }
    this.eventListeners.get(eventType)!.push(listener)
  }

  /**
   * 移除事件监听器
   */
  off(eventType: string, listener: SyncEventListener): void {
    const listeners = this.eventListeners.get(eventType)
    if (listeners) {
      const index = listeners.indexOf(listener)
      if (index > -1) {
        listeners.splice(index, 1)
      }
    }
  }

  /**
   * 触发事件
   */
  protected emitEvent(type: string, data: any): void {
    const listeners = this.eventListeners.get(type)
    if (listeners && listeners.length > 0) {
      const event: SyncEvent = { type: type as any, data }
      listeners.forEach(listener => {
        try {
          listener(event)
        } catch (error) {
          logger.error('[同步提供者] 事件监听器执行错误:', error)
        }
      })
    }
  }

  /**
   * 销毁同步提供者
   */
  destroy(): void {
    this.eventListeners.clear()
    this.status = SyncStatus.IDLE
    this.isInitialized = false
  }
}
