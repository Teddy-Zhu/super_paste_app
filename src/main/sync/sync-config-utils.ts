import { app } from 'electron'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import { SyncProviderType, SyncDirection, type ScheduledSyncConfig, type WebDAVConfig } from './types'
import logger from '../utils/logger'

/**
 * 同步配置工具类
 * 提供同步配置的加载、解析等共享功能
 */
export class SyncConfigUtils {
  /**
   * 从存储中加载基础同步配置
   * 直接读取配置，不进行校验
   */
  static loadBaseSyncConfig(): ScheduledSyncConfig {
    // 读取基础同步配置
    const enabled = store.get<boolean>(STORE_KEY.SYNC_ENABLED, false)
    const provider = store.get<string>(STORE_KEY.SYNC_PROVIDER, 'webdav')
    const interval = store.get<number>(STORE_KEY.SYNC_INTERVAL, 30)
    const direction = store.get<string>(STORE_KEY.SYNC_DIRECTION, 'bidirectional')

    // 读取 provider 配置参数
    let providerConfig: WebDAVConfig

    if (provider === 'webdav') {
      const webdavParams = store.get<any>(STORE_KEY.SYNC_PROVIDER_PARAMS.replace('%s', 'webdav'), {})
      logger.debug('[同步配置工具] WebDAV 配置参数:', webdavParams)
      providerConfig = {
        syncDirection: this.parseSyncDirection(direction),
        serverUrl: webdavParams.serverUrl || '',
        username: webdavParams.username || '',
        password: webdavParams.password || '',
        remotePath: webdavParams.remotePath || '/',
        timeout: webdavParams.timeout || 30000,
        useHttps: webdavParams.serverUrl?.startsWith('https') || false
      }
    } else {
      // 默认 WebDAV 配置
      providerConfig = {
        syncDirection: this.parseSyncDirection(direction),
        serverUrl: '',
        username: '',
        password: '',
        remotePath: '/',
        timeout: 30000,
        useHttps: false
      }
    }

    // 构建配置
    const config: ScheduledSyncConfig = {
      enabled: enabled,
      syncInterval: interval > 0 ? interval : 30,
      providerType: this.parseProviderType(provider),
      providerConfig: providerConfig,
      userDataPath: app.getPath('userData')
    }

    return config
  }

  /**
   * 从存储中加载完整的同步配置
   * 只有在配置完整且有效时才返回配置，否则返回 null
   */
  static loadSyncConfig(): ScheduledSyncConfig | null {
    try {
      // 读取基础同步配置
      const enabled = store.get<boolean>(STORE_KEY.SYNC_ENABLED, false)
      const provider = store.get<string>(STORE_KEY.SYNC_PROVIDER, '')
      const interval = store.get<number>(STORE_KEY.SYNC_INTERVAL, 30)
      const direction = store.get<string>(STORE_KEY.SYNC_DIRECTION, 'bidirectional')

      // 如果提供者未配置，返回 null（但不检查 enabled 状态）
      if (!provider) {
        return null
      }

      // 根据提供者类型读取具体配置
      let providerConfig: WebDAVConfig | null = null

      if (provider === 'webdav') {
        // 读取 WebDAV 配置参数
        const webdavParams = store.get<any>(STORE_KEY.SYNC_PROVIDER_PARAMS.replace('%s', 'webdav'), {})

        if (!webdavParams.serverUrl || !webdavParams.username || !webdavParams.password) {
          logger.warn('[同步配置工具] WebDAV 配置参数不完整')
          return null
        }

        providerConfig = {
          syncDirection: this.parseSyncDirection(direction),
          serverUrl: webdavParams.serverUrl,
          username: webdavParams.username,
          password: webdavParams.password,
          remotePath: webdavParams.remotePath || '/',
          timeout: webdavParams.timeout || 30000,
          useHttps: webdavParams.serverUrl?.startsWith('https') || false
        }
      }

      if (!providerConfig) {
        logger.warn(`[同步配置工具] 不支持的同步提供者: ${provider}`)
        return null
      }

      // 构建完整的定时同步配置
      const syncConfig: ScheduledSyncConfig = {
        enabled: enabled,
        syncInterval: interval,
        providerType: this.parseProviderType(provider),
        providerConfig: providerConfig,
        userDataPath: app.getPath('userData')
      }

      logger.debug('[同步配置工具] 成功加载同步配置:', {
        enabled: syncConfig.enabled,
        provider: provider,
        interval: interval,
        direction: direction
      })

      return syncConfig
    } catch (error) {
      logger.error('[同步配置工具] 加载同步配置失败:', error)
      return null
    }
  }

  /**
   * 解析同步方向字符串
   */
  static parseSyncDirection(direction: string): SyncDirection {
    switch (direction.toLowerCase()) {
      case 'upload':
        return SyncDirection.UPLOAD
      case 'download':
        return SyncDirection.DOWNLOAD
      case 'bidirectional':
      default:
        return SyncDirection.BIDIRECTIONAL
    }
  }

  /**
   * 解析同步提供者类型字符串
   */
  static parseProviderType(provider: string): SyncProviderType {
    switch (provider.toLowerCase()) {
      case 'webdav':
        return SyncProviderType.WEBDAV
      case 'ftp':
        return SyncProviderType.FTP
      case 'sftp':
        return SyncProviderType.SFTP
      case 'cloud_storage':
        return SyncProviderType.CLOUD_STORAGE
      default:
        return SyncProviderType.WEBDAV
    }
  }
}
