import { app } from 'electron'
import { join } from 'path'
import { DatabaseService } from '../services/database-service'
import store from '../config/store'
import { BaseSyncProvider } from './base-sync-provider'
import { WebDAVSyncProvider } from './webdav-sync-provider'
import {
  SyncStatus,
  SyncDirection,
  SyncProviderType
} from './types'
import type {
  SyncConfig,
  SyncResult,
  SyncEventListener,
  WebDAVConfig,
  SyncProviderConfig
} from './types'
import logger from '../utils/logger'

/**
 * 数据库和配置文件同步服务
 * 负责管理数据库文件和配置文件的同步
 */
export class SyncService {
  private static instance: SyncService
  private syncProvider: BaseSyncProvider | null = null
  private databaseService: DatabaseService
  private autoSyncTimer: NodeJS.Timeout | null = null
  private isAutoSyncEnabled = false

  // 需要同步的文件路径
  private readonly DATABASE_FILE_NAME = 'clipboard-history.db'
  private readonly CONFIG_FILE_NAME = 'super-paste-config.json'
  private readonly REMOTE_DB_PATH = '/super-paste/database/clipboard-history.db'
  private readonly REMOTE_CONFIG_PATH = '/super-paste/config/super-paste-config.json'

  private constructor() {
    this.databaseService = DatabaseService.getInstance()
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): SyncService {
    if (!SyncService.instance) {
      SyncService.instance = new SyncService()
    }
    return SyncService.instance
  }

  /**
   * 初始化同步服务
   */
  async initialize(): Promise<boolean> {
    try {
      // 加载同步配置
      const syncConfig = this.loadSyncConfig()
      if (syncConfig && syncConfig.enabled) {
        await this.configureSyncProvider(syncConfig.type, syncConfig.config)
        
        if (syncConfig.config.autoSync) {
          this.startAutoSync()
        }
      }

      logger.info('[同步服务] 初始化完成')
      return true
    } catch (error) {
      logger.error('[同步服务] 初始化失败:', error)
      return false
    }
  }

  /**
   * 配置同步提供者
   */
  async configureSyncProvider(type: SyncProviderType, config: SyncConfig): Promise<boolean> {
    try {
      // 销毁现有同步提供者
      if (this.syncProvider) {
        this.syncProvider.destroy()
        this.syncProvider = null
      }

      // 创建新的同步提供者
      switch (type) {
        case SyncProviderType.WEBDAV:
          this.syncProvider = new WebDAVSyncProvider(config as WebDAVConfig)
          break
        // 可以在这里添加其他同步提供者
        default:
          throw new Error(`不支持的同步提供者类型: ${type}`)
      }

      // 初始化同步提供者
      const initialized = await this.syncProvider.initialize()
      if (!initialized) {
        this.syncProvider = null
        return false
      }

      // 保存配置
      this.saveSyncConfig({
        type,
        config,
        enabled: true
      })

      logger.info(`[同步服务] 同步提供者配置成功: ${type}`)
      return true
    } catch (error) {
      logger.error('[同步服务] 配置同步提供者失败:', error)
      return false
    }
  }

  /**
   * 测试同步连接
   */
  async testConnection(): Promise<boolean> {
    if (!this.syncProvider) {
      logger.error('[同步服务] 同步提供者未配置')
      return false
    }

    try {
      return await this.syncProvider.testConnection()
    } catch (error) {
      logger.error('[同步服务] 测试连接失败:', error)
      return false
    }
  }

  /**
   * 执行完整同步
   */
  async sync(): Promise<SyncResult> {
    if (!this.syncProvider) {
      return {
        success: false,
        message: '同步提供者未配置',
        errors: ['同步提供者未配置']
      }
    }

    try {
      logger.info('[同步服务] 开始同步数据库和配置文件')
      
      const result: SyncResult = {
        success: true,
        message: '同步完成',
        uploadedFiles: [],
        downloadedFiles: [],
        errors: []
      }

      // 根据同步方向执行不同的同步逻辑
      const config = this.syncProvider.getConfig()
      switch (config.syncDirection) {
        case SyncDirection.UPLOAD:
          await this.syncUpload(result)
          break
        case SyncDirection.DOWNLOAD:
          await this.syncDownload(result)
          break
        case SyncDirection.BIDIRECTIONAL:
          await this.syncBidirectional(result)
          break
      }

      if (result.errors && result.errors.length > 0) {
        result.success = false
        result.message = `同步完成，但存在 ${result.errors.length} 个错误`
      }

      logger.info('[同步服务] 同步完成', result)
      return result
    } catch (error) {
      logger.error('[同步服务] 同步过程中发生错误:', error)
      return {
        success: false,
        message: `同步失败: ${error instanceof Error ? error.message : '未知错误'}`,
        errors: [error instanceof Error ? error.message : '未知错误']
      }
    }
  }

  /**
   * 执行上传同步
   */
  private async syncUpload(result: SyncResult): Promise<void> {
    const localDbPath = this.getLocalDatabasePath()
    const localConfigPath = this.getLocalConfigPath()

    // 上传数据库文件
    if (await this.fileExists(localDbPath)) {
      const uploaded = await this.syncProvider!.uploadFile(localDbPath, this.REMOTE_DB_PATH)
      if (uploaded) {
        result.uploadedFiles?.push(this.DATABASE_FILE_NAME)
      } else {
        result.errors?.push('数据库文件上传失败')
      }
    }

    // 上传配置文件
    if (await this.fileExists(localConfigPath)) {
      const uploaded = await this.syncProvider!.uploadFile(localConfigPath, this.REMOTE_CONFIG_PATH)
      if (uploaded) {
        result.uploadedFiles?.push(this.CONFIG_FILE_NAME)
      } else {
        result.errors?.push('配置文件上传失败')
      }
    }
  }

  /**
   * 执行下载同步
   */
  private async syncDownload(result: SyncResult): Promise<void> {
    const localDbPath = this.getLocalDatabasePath()
    const localConfigPath = this.getLocalConfigPath()

    // 下载数据库文件
    const dbExists = await this.syncProvider!.fileExists(this.REMOTE_DB_PATH)
    if (dbExists) {
      const downloaded = await this.syncProvider!.downloadFile(this.REMOTE_DB_PATH, localDbPath)
      if (downloaded) {
        result.downloadedFiles?.push(this.DATABASE_FILE_NAME)
        // 重新初始化数据库连接
        this.databaseService.initialize()
      } else {
        result.errors?.push('数据库文件下载失败')
      }
    }

    // 下载配置文件
    const configExists = await this.syncProvider!.fileExists(this.REMOTE_CONFIG_PATH)
    if (configExists) {
      const downloaded = await this.syncProvider!.downloadFile(this.REMOTE_CONFIG_PATH, localConfigPath)
      if (downloaded) {
        result.downloadedFiles?.push(this.CONFIG_FILE_NAME)
        // 重新加载配置
        this.reloadConfig()
      } else {
        result.errors?.push('配置文件下载失败')
      }
    }
  }

  /**
   * 执行双向同步
   */
  private async syncBidirectional(result: SyncResult): Promise<void> {
    // 数据库文件双向同步
    await this.syncFileBidirectional(
      this.getLocalDatabasePath(),
      this.REMOTE_DB_PATH,
      this.DATABASE_FILE_NAME,
      result
    )

    // 配置文件双向同步
    await this.syncFileBidirectional(
      this.getLocalConfigPath(),
      this.REMOTE_CONFIG_PATH,
      this.CONFIG_FILE_NAME,
      result
    )
  }

  /**
   * 双向同步单个文件
   */
  private async syncFileBidirectional(
    localPath: string,
    remotePath: string,
    fileName: string,
    result: SyncResult
  ): Promise<void> {
    try {
      const localExists = await this.fileExists(localPath)
      const remoteExists = await this.syncProvider!.fileExists(remotePath)

      if (!localExists && !remoteExists) {
        // 两边都不存在，跳过
        return
      }

      if (localExists && !remoteExists) {
        // 本地存在，远程不存在，上传
        const uploaded = await this.syncProvider!.uploadFile(localPath, remotePath)
        if (uploaded) {
          result.uploadedFiles?.push(fileName)
        } else {
          result.errors?.push(`${fileName} 上传失败`)
        }
      } else if (!localExists && remoteExists) {
        // 本地不存在，远程存在，下载
        const downloaded = await this.syncProvider!.downloadFile(remotePath, localPath)
        if (downloaded) {
          result.downloadedFiles?.push(fileName)
        } else {
          result.errors?.push(`${fileName} 下载失败`)
        }
      } else {
        // 两边都存在，比较修改时间
        const localStat = await this.getLocalFileStats(localPath)
        const remoteInfo = await this.syncProvider!.getFileInfo(remotePath)

        if (localStat && remoteInfo) {
          if (localStat.mtime > remoteInfo.lastModified) {
            // 本地更新，上传
            const uploaded = await this.syncProvider!.uploadFile(localPath, remotePath)
            if (uploaded) {
              result.uploadedFiles?.push(fileName)
            } else {
              result.errors?.push(`${fileName} 上传失败`)
            }
          } else if (remoteInfo.lastModified > localStat.mtime) {
            // 远程更新，下载
            const downloaded = await this.syncProvider!.downloadFile(remotePath, localPath)
            if (downloaded) {
              result.downloadedFiles?.push(fileName)
            } else {
              result.errors?.push(`${fileName} 下载失败`)
            }
          }
          // 如果修改时间相同，则跳过同步
        }
      }
    } catch (error) {
      result.errors?.push(`${fileName} 同步失败: ${error instanceof Error ? error.message : '未知错误'}`)
    }
  }

  /**
   * 启动自动同步
   */
  startAutoSync(): void {
    if (this.isAutoSyncEnabled) {
      return
    }

    const config = this.syncProvider?.getConfig()
    if (!config || !config.autoSync) {
      return
    }

    const interval = (config.syncInterval || 30) * 60 * 1000 // 转换为毫秒

    this.autoSyncTimer = setInterval(async () => {
      try {
        await this.sync()
      } catch (error) {
        logger.error('[同步服务] 自动同步失败:', error)
      }
    }, interval)

    this.isAutoSyncEnabled = true
    logger.info(`[同步服务] 自动同步已启动，间隔: ${config.syncInterval} 分钟`)
  }

  /**
   * 停止自动同步
   */
  stopAutoSync(): void {
    if (this.autoSyncTimer) {
      clearInterval(this.autoSyncTimer)
      this.autoSyncTimer = null
      this.isAutoSyncEnabled = false
      logger.info('[同步服务] 自动同步已停止')
    }
  }

  /**
   * 获取同步状态
   */
  getSyncStatus(): SyncStatus {
    return this.syncProvider?.getStatus() || SyncStatus.IDLE
  }

  /**
   * 添加同步事件监听器
   */
  addSyncEventListener(eventType: string, listener: SyncEventListener): void {
    if (this.syncProvider) {
      this.syncProvider.on(eventType, listener)
    }
  }

  /**
   * 移除同步事件监听器
   */
  removeSyncEventListener(eventType: string, listener: SyncEventListener): void {
    if (this.syncProvider) {
      this.syncProvider.off(eventType, listener)
    }
  }

  /**
   * 禁用同步
   */
  disableSync(): void {
    this.stopAutoSync()
    if (this.syncProvider) {
      this.syncProvider.destroy()
      this.syncProvider = null
    }
    
    // 更新配置
    const syncConfig = this.loadSyncConfig()
    if (syncConfig) {
      syncConfig.enabled = false
      store.set('sync_config', syncConfig)
    }

    logger.info('[同步服务] 同步已禁用')
  }

  /**
   * 获取本地数据库文件路径
   */
  private getLocalDatabasePath(): string {
    return join(app.getPath('userData'), this.DATABASE_FILE_NAME)
  }

  /**
   * 获取本地配置文件路径
   */
  private getLocalConfigPath(): string {
    return join(app.getPath('userData'), this.CONFIG_FILE_NAME)
  }

  /**
   * 检查本地文件是否存在
   */
  private async fileExists(filePath: string): Promise<boolean> {
    try {
      const { promises: fs } = await import('fs')
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * 获取本地文件状态
   */
  private async getLocalFileStats(filePath: string): Promise<{ mtime: Date } | null> {
    try {
      const { promises: fs } = await import('fs')
      const stats = await fs.stat(filePath)
      return { mtime: stats.mtime }
    } catch {
      return null
    }
  }

  /**
   * 重新加载配置
   */
  private reloadConfig(): void {
    // 这里可以触发配置重新加载的逻辑
    logger.info('[同步服务] 配置文件已重新加载')
  }

  /**
   * 加载同步配置
   */
  private loadSyncConfig(): (SyncProviderConfig & { enabled: boolean }) | null {
    return store.get('sync_config', null)
  }

  /**
   * 保存同步配置
   */
  private saveSyncConfig(config: SyncProviderConfig & { enabled: boolean }): void {
    store.set('sync_config', config)
  }

  /**
   * 销毁同步服务
   */
  destroy(): void {
    this.stopAutoSync()
    if (this.syncProvider) {
      this.syncProvider.destroy()
      this.syncProvider = null
    }
    logger.info('[同步服务] 已销毁')
  }
}

// 导出单例实例
export const syncService = SyncService.getInstance()
