import { BrowserWindow } from 'electron'
import clipboardDatabase from './clipboard-database'
import store from './store'
import { STORE_KEY } from './common'
import type { CleanupConfig, CleanupStats } from './types/clipboard-types'

/**
 * 剪贴板历史记录清理器
 * 根据配置的最大保存时间自动清理过期的历史记录
 */
class ClipboardHistoryCleaner {
  private static instance: ClipboardHistoryCleaner
  private config: CleanupConfig
  private cleanupTimer: NodeJS.Timeout | null = null
  private isRunning: boolean = false
  private memoryDataSyncCallback: ((deletedIds: string[]) => void) | null = null

  private constructor() {
    // 从 store 中获取配置
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)

    this.config = {
      maxKeepTime: maxKeepTime,
      cleanupInterval: 60 * 60 * 1000, // 默认每小时检查一次
      enableAutoCleanup: maxKeepTime !== -1 // 只有设置了时间限制才启用自动清理
    }

    console.log('[历史记录清理器] 初始化完成，配置:', this.config)
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
   * 设置内存数据同步回调函数
   * 当清理器删除数据库记录时，会调用此回调来同步删除内存中的数据
   */
  public setMemoryDataSyncCallback(callback: (deletedIds: string[]) => void): void {
    this.memoryDataSyncCallback = callback
    console.log('[历史记录清理器] 已设置内存数据同步回调')
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
    if (!this.config.enableAutoCleanup) {
      console.log('[历史记录清理器] 自动清理已禁用（maxKeepTime = -1）')
      return
    }

    if (this.cleanupTimer) {
      console.log('[历史记录清理器] 自动清理已在运行中')
      return
    }

    console.log('[历史记录清理器] 启动自动清理，检查间隔:', this.config.cleanupInterval / 1000, '秒')
    
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
      console.log('[历史记录清理器] 自动清理已停止')
    }
  }

  /**
   * 执行清理操作
   */
  public async performCleanup(): Promise<CleanupStats> {
    if (this.isRunning) {
      console.log('[历史记录清理器] 清理操作正在进行中，跳过本次清理')
      return {
        totalRecords: 0,
        expiredRecords: 0,
        deletedRecords: 0,
        cleanupTime: 0
      }
    }

    if (!this.config.enableAutoCleanup) {
      console.log('[历史记录清理器] 清理已禁用')
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
      console.log('[历史记录清理器] 开始执行清理操作')

      // 检查数据库连接
      if (!clipboardDatabase.isConnected()) {
        console.warn('[历史记录清理器] 数据库未连接，跳过清理')
        return {
          totalRecords: 0,
          expiredRecords: 0,
          deletedRecords: 0,
          cleanupTime: Date.now() - startTime
        }
      }

      // 获取清理前的总记录数
      const totalRecords = clipboardDatabase.getRecordCount()

      // 计算过期时间戳
      const expireTimestamp = this.calculateExpireTimestamp()
      
      // 执行清理
      const deletedRecords = this.deleteExpiredRecords(expireTimestamp)

      const cleanupTime = Date.now() - startTime
      const stats: CleanupStats = {
        totalRecords,
        expiredRecords: deletedRecords, // 这里简化处理，实际删除的就是过期的
        deletedRecords,
        cleanupTime
      }

      if (deletedRecords > 0) {
        console.log(`[历史记录清理器] 清理完成，删除了 ${deletedRecords} 条过期记录，耗时 ${cleanupTime}ms`)
        // 通知前端剪贴板历史已更新
        this.notifyFrontendHistoryUpdated()
      } else {
        console.log(`[历史记录清理器] 清理完成，没有过期记录需要删除，耗时 ${cleanupTime}ms`)
      }

      return stats
    } catch (error) {
      console.error('[历史记录清理器] 清理操作失败:', error)
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
   * 计算过期时间戳
   */
  private calculateExpireTimestamp(): number {
    const now = Date.now()
    const maxKeepTimeMs = this.config.maxKeepTime * 24 * 60 * 60 * 1000 // 转换为毫秒
    return now - maxKeepTimeMs
  }

  /**
   * 删除过期记录
   */
  private deleteExpiredRecords(expireTimestamp: number): number {
    try {
      // 使用数据库事务确保操作的原子性
      return clipboardDatabase.transaction(() => {
        // 先查询过期记录的ID列表（用于同步内存数据）
        const expiredRecords = this.getExpiredRecordIds(expireTimestamp)

        if (expiredRecords.length === 0) {
          return 0
        }

        // 删除过期记录
        const deletedCount = clipboardDatabase.deleteRecordsByTimestamp(expireTimestamp)

        // 如果删除成功且设置了同步回调，则调用回调同步内存数据
        if (deletedCount > 0 && this.memoryDataSyncCallback) {
          this.memoryDataSyncCallback(expiredRecords)
        }

        return deletedCount
      })
    } catch (error) {
      console.error('[历史记录清理器] 删除过期记录失败:', error)
      return 0
    }
  }

  /**
   * 获取过期记录的ID列表（排除有标签的记录）
   */
  private getExpiredRecordIds(expireTimestamp: number): string[] {
    try {
      if (!clipboardDatabase.isConnected()) {
        return []
      }

      const records = clipboardDatabase.queryRecords({
        orderBy: 'timestamp',
        orderDirection: 'ASC'
      })

      // 获取有标签的记录ID列表
      const taggedRecordIds = new Set(clipboardDatabase.getTaggedRecordIds())

      return records
        .filter(record => {
          // 过期且没有标签的记录才能被清理
          return record.timestamp < expireTimestamp && !taggedRecordIds.has(record.content_id)
        })
        .map(record => record.content_id)
    } catch (error) {
      console.error('[历史记录清理器] 获取过期记录ID失败:', error)
      return []
    }
  }

  /**
   * 从 store 更新配置
   */
  public updateConfigFromStore(): void {
    const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)
    const oldConfig = { ...this.config }
    
    this.config.maxKeepTime = maxKeepTime
    this.config.enableAutoCleanup = maxKeepTime !== -1

    console.log('[历史记录清理器] 从 store 更新配置:', {
      old: oldConfig,
      new: this.config
    })

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
   * 销毁清理器（应用退出时调用）
   */
  public destroy(): void {
    this.stopAutoCleanup()
    console.log('[历史记录清理器] 清理器已销毁')
  }
}

// 导出单例实例
export default ClipboardHistoryCleaner.getInstance()
export { ClipboardHistoryCleaner }
