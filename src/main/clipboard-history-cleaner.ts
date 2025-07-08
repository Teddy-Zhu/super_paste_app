import { BrowserWindow } from 'electron'
import { ClipboardHistoryCleanerManager } from './workers/clipboard-history-cleaner-manager'
import store from './store'
import { STORE_KEY } from './common'
import type { CleanupConfig, CleanupStats } from './types/clipboard-types'
import clipboardHistoryCleanerWorkerPath from './workers/clipboard-history-cleaner.worker?modulePath'

/**
 * 剪贴板历史记录清理器（主线程接口）
 * 通过 Worker Thread 执行清理操作以提升性能
 */
class ClipboardHistoryCleaner {
  private static instance: ClipboardHistoryCleaner
  private workerManager: ClipboardHistoryCleanerManager | null = null
  private isInitialized: boolean = false

  private constructor() {
    console.log('[历史记录清理器] 初始化主线程接口')
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ClipboardHistoryCleaner {
    if (!ClipboardHistoryCleaner.instance) {
      ClipboardHistoryCleaner.instance = new ClipboardHistoryCleaner()
    }
    return ClipboardHistoryCleaner.instance
  }

  /**
   * 初始化 Worker
   */
  private async initializeWorker(): Promise<void> {
    if (this.workerManager) {
      return
    }

    try {
      // 创建 Worker 管理器
      this.workerManager = new ClipboardHistoryCleanerManager(clipboardHistoryCleanerWorkerPath)

      // 启动 Worker
      await this.workerManager.start()

      // 初始化 Worker
      await this.workerManager.initialize()

      // 从 store 更新配置
      this.updateConfigFromStore()

      this.isInitialized = true
      console.log('[历史记录清理器] Worker 初始化完成')
    } catch (error) {
      console.error('[历史记录清理器] Worker 初始化失败:', error)
      throw error
    }
  }

  /**
   * 确保 Worker 已初始化
   */
  private async ensureWorkerInitialized(): Promise<void> {
    if (!this.isInitialized) {
      await this.initializeWorker()
    }
  }

  /**
   * 设置内存数据同步回调函数
   * 当清理器删除数据库记录时，会调用此回调来同步删除内存中的数据
   */
  public setMemoryDataSyncCallback(callback: (deletedIds: string[]) => void): void {
    this.ensureWorkerInitialized()
      .then(() => {
        if (this.workerManager) {
          this.workerManager.setMemoryDataSyncCallback(callback)
          console.log('[历史记录清理器] 已设置内存数据同步回调')
        }
      })
      .catch((error) => {
        console.error('[历史记录清理器] 设置内存同步回调失败:', error)
      })
  }

  /**
   * 通知前端剪贴板历史已更新
   */
  private notifyFrontendHistoryUpdated(): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', {
            timestamp: Date.now(),
            source: 'cleanup'
          })
        }
      })

      console.log('[历史记录清理器] 已通知前端剪贴板历史更新')
    } catch (error) {
      console.error('[历史记录清理器] 通知前端失败:', error)
    }
  }

  /**
   * 启动自动清理
   */
  public startAutoCleanup(): void {
    this.ensureWorkerInitialized()
      .then(() => {
        if (this.workerManager) {
          return this.workerManager.startAutoCleanup()
        }
        return Promise.resolve()
      })
      .catch((error) => {
        console.error('[历史记录清理器] 启动自动清理失败:', error)
      })
  }

  /**
   * 停止自动清理
   */
  public stopAutoCleanup(): void {
    if (this.workerManager) {
      this.workerManager.stopAutoCleanup().catch((error) => {
        console.error('[历史记录清理器] 停止自动清理失败:', error)
      })
    }
  }

  /**
   * 执行清理操作
   */
  public async performCleanup(): Promise<CleanupStats> {
    try {
      await this.ensureWorkerInitialized()

      if (!this.workerManager) {
        throw new Error('Worker 未初始化')
      }

      const stats = await this.workerManager.performCleanup()

      // 如果有记录被删除，通知前端更新
      if (stats.deletedRecords > 0) {
        this.notifyFrontendHistoryUpdated()
      }

      return stats
    } catch (error) {
      console.error('[历史记录清理器] 执行清理操作失败:', error)
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }
  }

  /**
   * 从 store 更新配置
   */
  public updateConfigFromStore(): void {
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)

    console.log('[历史记录清理器] 从 store 更新配置，maxKeepTime:', maxKeepTime)

    // 更新 Worker 配置
    this.ensureWorkerInitialized()
      .then(() => {
        if (this.workerManager) {
          return this.workerManager.updateConfig({ maxKeepTime })
        }
        return Promise.resolve()
      })
      .catch((error) => {
        console.error('[历史记录清理器] 更新配置失败:', error)
      })
  }

  /**
   * 获取当前配置
   */
  public getConfig(): CleanupConfig {
    if (this.workerManager) {
      // 异步获取配置，但为了兼容性返回默认配置
      this.workerManager
        .getConfig()
        .then((config) => {
          console.log('[历史记录清理器] 当前配置:', config)
        })
        .catch((error) => {
          console.error('[历史记录清理器] 获取配置失败:', error)
        })
    }

    // 返回默认配置以保持兼容性
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)
    return {
      maxKeepTime,
      cleanupInterval: 60 * 60 * 1000,
      enableAutoCleanup: maxKeepTime !== -1
    }
  }

  /**
   * 销毁清理器（应用退出时调用）
   */
  public destroy(): void {
    if (this.workerManager) {
      this.workerManager.destroy().catch((error) => {
        console.error('[历史记录清理器] 销毁清理器失败:', error)
      })
    }
    console.log('[历史记录清理器] 清理器已销毁')
  }
}

// 导出单例实例
export default ClipboardHistoryCleaner.getInstance()
export { ClipboardHistoryCleaner }
