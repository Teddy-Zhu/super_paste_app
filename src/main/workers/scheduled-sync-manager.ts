import { join } from 'path'
import { BaseWorkerManager } from './base-worker-manager'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import type {
  SyncConfig,
  SyncProviderType,
  SyncResult,
  SyncStatus
} from '../sync/types'
import logger from '../utils/logger'

/**
 * 定时同步任务配置接口
 */
interface ScheduledSyncConfig {
  /** 是否启用定时同步 */
  enabled: boolean
  /** 同步间隔（分钟） */
  syncInterval: number
  /** 同步提供者类型 */
  providerType: SyncProviderType
  /** 同步提供者配置 */
  providerConfig: SyncConfig
  /** 用户数据目录路径 */
  userDataPath: string
  /** 配置存储接口（可选） */
  configStore?: {
    get: (key: string, defaultValue?: any) => any
    set: (key: string, value: any) => void
  }
}

/**
 * 定时同步状态接口
 */
interface ScheduledSyncStatus {
  isRunning: boolean
  lastSyncTime: number
  nextSyncTime: number
  syncStatus: SyncStatus
  config: ScheduledSyncConfig | null
}

/**
 * 定时同步任务 Worker 管理器
 * 负责管理定时同步任务 Worker 的生命周期和通信
 */
export class ScheduledSyncManager extends BaseWorkerManager {
  private static instance: ScheduledSyncManager

  constructor() {
    const config: WorkerConfig = {
      name: '定时同步任务管理器',
      workerPath: join(__dirname, 'scheduled-sync.worker.js'),
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
   */
  public async initialize(config: ScheduledSyncConfig): Promise<boolean> {
    try {
      if (!this.isRunning()) {
        await this.start()
      }

      const result = await this.call<boolean>('initialize', config)
      
      if (result) {
        logger.info(`[${this.config.name}] 定时同步初始化成功`)
      } else {
        logger.warn(`[${this.config.name}] 定时同步初始化失败或未启用`)
      }

      return result
    } catch (error) {
      logger.error(`[${this.config.name}] 初始化定时同步失败:`, error)
      return false
    }
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
    // 定时同步 Worker 目前不需要向主线程发送请求
    // 如果将来需要，可以在这里处理
    logger.debug(`[${this.config.name}] 收到 Worker 请求:`, request.method)
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

    // 同步开始事件
    this.on('sync-started', (data) => {
      logger.debug(`[${this.config.name}] 开始执行同步任务`)
    })

    // 同步完成事件
    this.on('sync-completed', (data) => {
      logger.info(`[${this.config.name}] 同步任务完成`)
    })

    // 同步失败事件
    this.on('sync-failed', (data) => {
      logger.warn(`[${this.config.name}] 同步任务失败:`, data.result?.message)
    })

    // 同步错误事件
    this.on('sync-error', (data) => {
      logger.error(`[${this.config.name}] 同步任务发生错误:`, data.error)
    })

    // 手动同步事件
    this.on('manual-sync-started', () => {
      logger.debug(`[${this.config.name}] 开始执行手动同步`)
    })

    this.on('manual-sync-completed', (data) => {
      logger.info(`[${this.config.name}] 手动同步完成`)
    })

    this.on('manual-sync-failed', (data) => {
      logger.warn(`[${this.config.name}] 手动同步失败:`, data.result?.message)
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
