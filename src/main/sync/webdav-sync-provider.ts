import { createClient, WebDAVClient, FileStat } from 'webdav'
import { promises as fs } from 'fs'
import { dirname } from 'path'
import { BaseSyncProvider } from './base-sync-provider'
import type { SyncFileInfo, SyncResult, WebDAVConfig } from './types'
import { SyncProviderType } from './types'
import logger from '../utils/logger'
/**
 * WebDAV 同步提供者
 * 实现基于 WebDAV 协议的文件同步功能
 */
export class WebDAVSyncProvider extends BaseSyncProvider {
  private client: WebDAVClient | null = null
  private webdavConfig: WebDAVConfig

  constructor(config: WebDAVConfig) {
    super(config)
    this.webdavConfig = config
  }

  name(): SyncProviderType {
    return SyncProviderType.WEBDAV
  }

  /**
   * 初始化 WebDAV 客户端
   */
  async initialize(): Promise<boolean> {
    try {
      // 创建 WebDAV 连接
      this.client = createClient(this.webdavConfig.serverUrl, {
        username: this.webdavConfig.username,
        password: this.webdavConfig.password
      })

      // 测试连接
      const connectionTest = await this.testConnection()
      if (!connectionTest) {
        logger.error('[WebDAV同步] 连接测试失败')
        return false
      }

      // 确保远程目录存在
      await this.ensureRemoteDirectory(this.webdavConfig.remotePath)

      this.isInitialized = true
      logger.info('[WebDAV同步] 初始化成功')
      return true
    } catch (error) {
      logger.error('[WebDAV同步] 初始化失败:', error)
      return false
    }
  }

  /**
   * 测试 WebDAV 连接
   */
  async testConnection(): Promise<boolean> {
    if (!this.client) {
      return false
    }

    try {
      await this.client.getDirectoryContents('/')
      logger.debug('[WebDAV同步] 连接测试成功')
      return true
    } catch (error) {
      logger.error('[WebDAV同步] 连接测试失败:', error)
      return false
    }
  }

  /**
   * 上传文件到 WebDAV 服务器
   */
  async uploadFile(localPath: string, remotePath: string): Promise<boolean> {
    try {
      // 检查本地文件是否存在
      const exists = await this.fileExistsLocal(localPath)
      if (!exists) {
        logger.error(`[WebDAV同步] 本地文件不存在: ${localPath}`)
        return false
      }

      // 读取本地文件
      const fileContent = await fs.readFile(localPath)

      // 使用 uploadFileWithData 方法上传
      const uploaded = await this.uploadFileWithData(fileContent, remotePath)
      if (uploaded) {
        logger.debug(`[WebDAV同步] 文件上传成功: ${localPath} -> ${remotePath}`)
      }
      return uploaded
    } catch (error) {
      logger.error(`[WebDAV同步] 文件上传失败: ${localPath}`, error)
      return false
    }
  }

  /**
   * 直接上传文件内容到 WebDAV 服务器
   */
  async uploadFileWithData(fileContent: string | Buffer, remotePath: string): Promise<boolean> {
    if (!this.client) {
      logger.error('[WebDAV同步] 客户端未初始化')
      return false
    }

    try {
      // 确保远程目录存在
      const remoteDir = dirname(remotePath)
      await this.ensureRemoteDirectory(remoteDir)

      // 上传文件内容
      await this.client.putFileContents(remotePath, fileContent)
      logger.debug(`[WebDAV同步] 文件内容上传成功: ${remotePath}`)
      this.emitEvent('file-synced', {
        action: 'upload',
        remotePath,
        timestamp: new Date()
      })
      return true
    } catch (error) {
      logger.error(`[WebDAV同步] 文件内容上传失败: ${remotePath}`, error)
      return false
    }
  }

  /**
   * 从 WebDAV 服务器下载文件
   */
  async downloadFile(remotePath: string, localPath: string): Promise<boolean> {
    if (!this.client) {
      logger.error('[WebDAV同步] 客户端未初始化')
      return false
    }

    try {
      // 检查远程文件是否存在
      const exists = await this.fileExists(remotePath)
      if (!exists) {
        logger.error(`[WebDAV同步] 远程文件不存在: ${remotePath}`)
        return false
      }

      // 确保本地目录存在
      const localDir = dirname(localPath)
      await this.ensureLocalDirectory(localDir)

      // 下载文件
      const fileContent = await this.client.getFileContents(remotePath)
      await fs.writeFile(localPath, fileContent as Buffer)

      logger.debug(`[WebDAV同步] 文件下载成功: ${remotePath} -> ${localPath}`)
      this.emitEvent('file-synced', {
        action: 'download',
        remotePath,
        localPath,
        timestamp: new Date()
      })
      return true
    } catch (error) {
      logger.error(`[WebDAV同步] 文件下载失败: ${remotePath}`, error)
      return false
    }
  }

  /**
   * 删除远程文件
   */
  async deleteFile(remotePath: string): Promise<boolean> {
    if (!this.client) {
      logger.error('[WebDAV同步] 客户端未初始化')
      return false
    }

    try {
      await this.client.deleteFile(remotePath)
      logger.debug(`[WebDAV同步] 文件删除成功: ${remotePath}`)
      return true
    } catch (error) {
      logger.error(`[WebDAV同步] 文件删除失败: ${remotePath}`, error)
      return false
    }
  }

  /**
   * 列出远程目录中的文件
   */
  async listFiles(remotePath?: string): Promise<SyncFileInfo[]> {
    if (!this.client) {
      logger.error('[WebDAV同步] 客户端未初始化')
      return []
    }

    const targetPath = remotePath || this.webdavConfig.remotePath

    try {
      const files = (await this.client.getDirectoryContents(targetPath)) as FileStat[]
      const fileInfos: SyncFileInfo[] = []

      for (const item of files) {
        if (item.type === 'file') {
          fileInfos.push({
            path: item.filename,
            lastModified: new Date(item.lastmod),
            size: item.size
          })
        }
      }

      return fileInfos
    } catch (error) {
      logger.error(`[WebDAV同步] 列出文件失败: ${targetPath}`, error)
      return []
    }
  }

  /**
   * 检查远程文件是否存在
   */
  async fileExists(remotePath: string): Promise<boolean> {
    if (!this.client) {
      return false
    }

    try {
      return await this.client.exists(remotePath)
    } catch (error) {
      return false
    }
  }

  /**
   * 获取远程文件信息
   */
  async getFileInfo(remotePath: string): Promise<SyncFileInfo | null> {
    if (!this.client) {
      return null
    }

    try {
      const stat = (await this.client.stat(remotePath)) as FileStat
      if (stat.type === 'file') {
        return {
          path: stat.filename,
          lastModified: new Date(stat.lastmod),
          size: stat.size
        }
      }
      return null
    } catch (error) {
      logger.error(`[WebDAV同步] 获取文件信息失败: ${remotePath}`, error)
      return null
    }
  }

  /**
   * 执行具体的同步逻辑
   */
  protected async performSync(): Promise<SyncResult> {
    const result: SyncResult = {
      success: true,
      message: '同步完成',
      uploadedFiles: [],
      downloadedFiles: [],
      errors: []
    }

    try {
      // 根据配置执行不同的同步方向
      switch (this.config.syncDirection) {
        case 'upload':
          await this.performUploadSync(result)
          break
        case 'download':
          await this.performDownloadSync(result)
          break
        case 'bidirectional':
          await this.performBidirectionalSync(result)
          break
      }

      if (result.errors && result.errors.length > 0) {
        result.success = false
        result.message = `同步完成，但存在 ${result.errors.length} 个错误`
      }
    } catch (error) {
      result.success = false
      result.message = `同步失败: ${error instanceof Error ? error.message : '未知错误'}`
      result.errors = result.errors || []
      result.errors.push(error instanceof Error ? error.message : '未知错误')
    }

    return result
  }

  /**
   * 执行上传同步
   */
  private async performUploadSync(_result: SyncResult): Promise<void> {
    // 这里需要根据具体需求实现数据库和配置文件的上传逻辑
    // 暂时先实现基础框架
    logger.info('[WebDAV同步] 执行上传同步')

    // TODO: 实现具体的文件上传逻辑
    // 例如: 上传数据库文件、配置文件等
  }

  /**
   * 执行下载同步
   */
  private async performDownloadSync(_result: SyncResult): Promise<void> {
    // 这里需要根据具体需求实现数据库和配置文件的下载逻辑
    logger.info('[WebDAV同步] 执行下载同步')

    // TODO: 实现具体的文件下载逻辑
  }

  /**
   * 执行双向同步
   */
  private async performBidirectionalSync(_result: SyncResult): Promise<void> {
    // 这里需要根据具体需求实现双向同步逻辑
    logger.info('[WebDAV同步] 执行双向同步')

    // TODO: 实现具体的双向同步逻辑
    // 需要比较本地和远程文件的修改时间，决定同步方向
  }

  /**
   * 确保远程目录存在
   */
  private async ensureRemoteDirectory(remotePath: string): Promise<void> {
    if (!this.client || !remotePath || remotePath === '/') {
      return
    }

    try {
      const exists = await this.client.exists(remotePath)
      if (!exists) {
        // 目录不存在，尝试创建
        await this.client.createDirectory(remotePath, { recursive: true })
        logger.debug(`[WebDAV同步] 创建远程目录: ${remotePath}`)
      }
    } catch (error) {
      logger.error(`[WebDAV同步] 创建远程目录失败: ${remotePath}`, error)
      throw error
    }
  }

  /**
   * 确保本地目录存在
   */
  private async ensureLocalDirectory(localPath: string): Promise<void> {
    try {
      await fs.mkdir(localPath, { recursive: true })
    } catch (error) {
      logger.error(`[WebDAV同步] 创建本地目录失败: ${localPath}`, error)
      throw error
    }
  }

  /**
   * 检查本地文件是否存在
   */
  private async fileExistsLocal(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * 销毁 WebDAV 客户端
   */
  destroy(): void {
    super.destroy()
    this.client = null
    logger.debug('[WebDAV同步] 客户端已销毁')
  }
}
