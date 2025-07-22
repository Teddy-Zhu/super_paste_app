/**
 * 同步功能集成示例
 * 展示如何在主进程中集成和使用同步功能
 */

import { syncService } from './sync-service'
import { SyncProviderType, SyncDirection } from './types'
import logger from '../utils/logger'

/**
 * 初始化同步功能
 * 在应用启动时调用
 */
export async function initializeSync(): Promise<void> {
  try {
    logger.info('[同步集成] 开始初始化同步功能')
    
    // 初始化同步服务
    const initialized = await syncService.initialize()
    if (!initialized) {
      logger.error('[同步集成] 同步服务初始化失败')
      return
    }

    // 添加事件监听器
    setupSyncEventListeners()
    
    logger.info('[同步集成] 同步功能初始化完成')
  } catch (error) {
    logger.error('[同步集成] 同步功能初始化异常:', error)
  }
}

/**
 * 配置 WebDAV 同步
 * 用于设置 WebDAV 同步参数
 */
export async function setupWebDAVSync(options: {
  serverUrl: string
  username: string
  password: string
  remotePath?: string
  autoSync?: boolean
  syncInterval?: number
  syncDirection?: SyncDirection
}): Promise<boolean> {
  try {
    logger.info('[同步集成] 配置 WebDAV 同步')
    
    const webdavConfig = {
      enabled: true,
      autoSync: options.autoSync ?? true,
      syncInterval: options.syncInterval ?? 30, // 默认30分钟
      syncDirection: options.syncDirection ?? SyncDirection.BIDIRECTIONAL,
      serverUrl: options.serverUrl,
      username: options.username,
      password: options.password,
      remotePath: options.remotePath ?? '/super-paste',
      timeout: 30000,
      useHttps: options.serverUrl.startsWith('https')
    }

    // 配置同步提供者
    const configured = await syncService.configureSyncProvider(
      SyncProviderType.WEBDAV,
      webdavConfig
    )

    if (!configured) {
      logger.error('[同步集成] WebDAV 同步配置失败')
      return false
    }

    // 测试连接
    const connectionOk = await syncService.testConnection()
    if (!connectionOk) {
      logger.error('[同步集成] WebDAV 连接测试失败')
      return false
    }

    logger.info('[同步集成] WebDAV 同步配置成功')
    return true
  } catch (error) {
    logger.error('[同步集成] WebDAV 同步配置异常:', error)
    return false
  }
}

/**
 * 执行手动同步
 */
export async function performManualSync(): Promise<boolean> {
  try {
    logger.info('[同步集成] 开始手动同步')
    
    const result = await syncService.sync()
    
    if (result.success) {
      logger.info('[同步集成] 同步完成:', {
        message: result.message,
        uploadedFiles: result.uploadedFiles,
        downloadedFiles: result.downloadedFiles
      })
      return true
    } else {
      logger.error('[同步集成] 同步失败:', {
        message: result.message,
        errors: result.errors
      })
      return false
    }
  } catch (error) {
    logger.error('[同步集成] 手动同步异常:', error)
    return false
  }
}

/**
 * 设置同步事件监听器
 */
function setupSyncEventListeners(): void {
  // 监听同步状态变化
  syncService.addSyncEventListener('status-changed', (event) => {
    logger.debug('[同步集成] 同步状态变化:', event.data)
    
    // 可以在这里通知渲染进程状态变化
    // 例如: mainWindow.webContents.send('sync-status-changed', event.data)
  })

  // 监听文件同步事件
  syncService.addSyncEventListener('file-synced', (event) => {
    logger.debug('[同步集成] 文件同步完成:', event.data)
    
    // 可以在这里显示同步进度
    // 例如: mainWindow.webContents.send('file-synced', event.data)
  })

  // 监听配置更新事件
  syncService.addSyncEventListener('config-updated', (event) => {
    logger.debug('[同步集成] 同步配置已更新:', event.data)
  })

  // 监听错误事件
  syncService.addSyncEventListener('error', (event) => {
    logger.error('[同步集成] 同步错误:', event.data)
    
    // 可以在这里显示错误通知
    // 例如: showErrorNotification(event.data)
  })
}

/**
 * 获取同步状态信息
 */
export function getSyncInfo(): {
  status: string
  isAutoSyncEnabled: boolean
} {
  return {
    status: syncService.getSyncStatus(),
    isAutoSyncEnabled: syncService['isAutoSyncEnabled'] || false
  }
}

/**
 * 控制自动同步
 */
export function toggleAutoSync(enable: boolean): void {
  if (enable) {
    syncService.startAutoSync()
    logger.info('[同步集成] 自动同步已启用')
  } else {
    syncService.stopAutoSync()
    logger.info('[同步集成] 自动同步已禁用')
  }
}

/**
 * 禁用同步功能
 */
export function disableSync(): void {
  syncService.disableSync()
  logger.info('[同步集成] 同步功能已禁用')
}

/**
 * 应用关闭时的清理
 */
export function cleanupSync(): void {
  syncService.destroy()
  logger.info('[同步集成] 同步功能已清理')
}

/**
 * IPC 处理器示例
 * 可以在 ipc-handlers.ts 中使用这些函数
 */
export const syncIPCHandlers = {
  // 配置 WebDAV 同步
  'sync:setup-webdav': async (_event: any, options: any) => {
    return await setupWebDAVSync(options)
  },

  // 执行手动同步
  'sync:manual-sync': async () => {
    return await performManualSync()
  },

  // 测试连接
  'sync:test-connection': async () => {
    return await syncService.testConnection()
  },

  // 获取同步状态
  'sync:get-status': () => {
    return getSyncInfo()
  },

  // 控制自动同步
  'sync:toggle-auto-sync': (_event: any, enable: boolean) => {
    toggleAutoSync(enable)
    return true
  },

  // 禁用同步
  'sync:disable': () => {
    disableSync()
    return true
  }
}
