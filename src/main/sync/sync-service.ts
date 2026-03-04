import { join } from 'path'
import { BaseSyncProvider } from './base-sync-provider'
import { WebDAVSyncProvider } from './webdav-sync-provider'
import { SyncStatus, SyncDirection, SyncProviderType } from './types'
import type { SyncProviderConfig, SyncResult, SyncEventListener, WebDAVConfig } from './types'
import logger from '../utils/worker-logger'
import { HashUtils } from '../utils/common'

/**
 * 数据库和配置文件同步服务
 * 专注于同步业务逻辑，完全由外部控制
 * 优化后可在 worker 中独立运行
 */
export class SyncService {
  private syncProvider: BaseSyncProvider | null = null
  private userDataPath: string = ''
  private onSyncRequiresRestart?: (data: { downloadedFiles: string[]; message: string }) => void

  // 需要同步的文件路径
  private readonly DATABASE_FILE_NAME = 'clipboard-history.db'
  private readonly CONFIG_FILE_NAME = 'config.json'
  private readonly REMOTE_DB_PATH = './clipboard-history.db'
  private readonly REMOTE_CONFIG_PATH = './config.json'

  constructor(
    userDataPath: string,
    options?: { onSyncRequiresRestart?: (data: { downloadedFiles: string[]; message: string }) => void }
  ) {
    this.userDataPath = userDataPath
    this.onSyncRequiresRestart = options?.onSyncRequiresRestart
  }

  /**
   * 初始化同步提供者
   * 由外部直接传入配置，不再从存储加载
   */
  async initialize(
    providerType: SyncProviderType,
    providerConfig: SyncProviderConfig
  ): Promise<boolean> {
    try {
      const success = await this.configureSyncProvider(providerType, providerConfig)

      if (success) {
        logger.info('[同步服务] 初始化完成')
      } else {
        logger.error('[同步服务] 初始化失败')
      }

      return success
    } catch (error) {
      logger.error('[同步服务] 初始化失败:', error)
      return false
    }
  }

  /**
   * 配置同步提供者
   */
  async configureSyncProvider(
    type: SyncProviderType,
    config: SyncProviderConfig
  ): Promise<boolean> {
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

      // 不再保存配置，由外部管理

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

      // 检查是否有文件被下载，如果有则需要通知前端重启
      if (result.downloadedFiles && result.downloadedFiles.length > 0) {
        logger.info('[同步服务] 检测到文件已下载，通知前端重启应用')
        this.onSyncRequiresRestart?.({
          downloadedFiles: result.downloadedFiles,
          message: `已下载 ${result.downloadedFiles.length} 个文件，需要重启应用以应用更改`
        })
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
    const config = this.syncProvider!.getConfig()

    // 上传数据库文件（除非设置了仅同步配置）
    if (!config.configOnly && (await this.fileExists(localDbPath))) {
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

    // 上传版本信息文件
    await this.uploadVersionFile(result)
  }

  /**
   * 执行下载同步
   */
  private async syncDownload(result: SyncResult): Promise<void> {
    const localDbPath = this.getLocalDatabasePath()
    const localConfigPath = this.getLocalConfigPath()
    const config = this.syncProvider!.getConfig()

    // 下载数据库文件（除非设置了仅同步配置）
    if (!config.configOnly) {
      const dbExists = await this.syncProvider!.fileExists(this.REMOTE_DB_PATH)
      if (dbExists) {
        const downloaded = await this.syncProvider!.downloadFile(this.REMOTE_DB_PATH, localDbPath)
        if (downloaded) {
          result.downloadedFiles?.push(this.DATABASE_FILE_NAME)
          // 数据库文件已下载，在 worker 中无需重新初始化数据库连接
        } else {
          result.errors?.push('数据库文件下载失败')
        }
      }
    }

    // 下载配置文件
    const configExists = await this.syncProvider!.fileExists(this.REMOTE_CONFIG_PATH)
    if (configExists) {
      const downloaded = await this.syncProvider!.downloadFile(
        this.REMOTE_CONFIG_PATH,
        localConfigPath
      )
      if (downloaded) {
        result.downloadedFiles?.push(this.CONFIG_FILE_NAME)
        // 配置文件已下载，在 worker 中无需重新加载配置
      } else {
        result.errors?.push('配置文件下载失败')
      }
    }
  }

  /**
   * 执行双向同步
   */
  private async syncBidirectional(result: SyncResult): Promise<void> {
    const config = this.syncProvider!.getConfig()

    // 数据库文件双向同步（除非设置了仅同步配置）
    if (!config.configOnly) {
      await this.syncFileBidirectional(
        this.getLocalDatabasePath(),
        this.REMOTE_DB_PATH,
        this.DATABASE_FILE_NAME,
        result
      )
    }

    // 配置文件双向同步
    await this.syncFileBidirectional(
      this.getLocalConfigPath(),
      this.REMOTE_CONFIG_PATH,
      this.CONFIG_FILE_NAME,
      result
    )

    // 如果有文件被上传，则上传版本信息文件
    if (result.uploadedFiles && result.uploadedFiles.length > 0) {
      await this.uploadVersionFile(result)
    }
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
      result.errors?.push(
        `${fileName} 同步失败: ${error instanceof Error ? error.message : '未知错误'}`
      )
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
   * 获取本地数据库文件路径
   */
  private getLocalDatabasePath(): string {
    if (!this.userDataPath) {
      throw new Error('用户数据路径未配置')
    }
    return join(this.userDataPath, this.DATABASE_FILE_NAME)
  }

  /**
   * 获取本地配置文件路径
   */
  private getLocalConfigPath(): string {
    if (!this.userDataPath) {
      throw new Error('用户数据路径未配置')
    }
    return join(this.userDataPath, this.CONFIG_FILE_NAME)
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
   * 上传版本信息文件
   */
  private async uploadVersionFile(result: SyncResult): Promise<void> {
    try {
      const config = this.syncProvider!.getConfig()
      const localDbPath = this.getLocalDatabasePath()
      const localConfigPath = this.getLocalConfigPath()

      // 计算配置文件hash
      const configFileHash = await HashUtils.generateFileHash(localConfigPath)

      // 创建版本信息
      const versionInfo: any = {
        lastUpdateTime: Date.now(),
        files: {
          config: {
            hash: configFileHash,
            path: this.CONFIG_FILE_NAME
          }
        }
      }

      // 如果不是仅同步配置模式，包含数据库文件信息
      if (!config.configOnly) {
        const dbFileHash = await HashUtils.generateFileHash(localDbPath)
        versionInfo.files.database = {
          hash: dbFileHash,
          path: this.DATABASE_FILE_NAME
        }
      }

      // 直接上传文件内容，无需创建临时文件
      const versionContent = JSON.stringify(versionInfo, null, 2)
      const uploaded = await this.syncProvider!.uploadFileWithData(versionContent, './version.txt')
      if (uploaded) {
        result.uploadedFiles?.push('version.txt')
        logger.info('[同步服务] 版本信息文件上传成功')
      } else {
        result.errors?.push('版本信息文件上传失败')
      }
    } catch (error) {
      logger.error('[同步服务] 创建或上传版本信息文件失败:', error)
      result.errors?.push('版本信息文件处理失败')
    }
  }

  /**
   * 销毁同步服务
   */
  destroy(): void {
    if (this.syncProvider) {
      this.syncProvider.destroy()
      this.syncProvider = null
    }
    logger.info('[同步服务] 已销毁')
  }
}
