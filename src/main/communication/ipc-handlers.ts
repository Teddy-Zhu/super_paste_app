import { ipcMain } from 'electron'
import { app } from 'electron'
import { mainWindowManager, overlayWindowManager } from '../ui/windows/window-managers'
import { windowStateManager, OverlayState } from '../ui/windows/window-state'
import { globalShortcutManager } from '../system/global-shortcut-manager'
import autoStartManager from '../utils/auto-start-manager'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import { debounceExecution } from '../utils/common'
import { MAX_HISTORY_SIZE_DEBOUNCE_DELAY, MAX_KEEP_TIME_DEBOUNCE_DELAY } from '../config/constants'
import clipboardManager from '../clipboard/clipboard-manager'
import clipboardHistoryCleaner from '../clipboard/clipboard-history-cleaner'
import { scheduledSyncManager } from '../workers/scheduled-sync-manager'
import type { ScheduledSyncConfig } from '../sync/types'
import { SyncStatus } from '../sync/types'
import { SyncConfigUtils } from '../sync/sync-config-utils'
import logger from '../utils/logger'

/**
 * IPC 事件处理器管理器
 */
export class IPCHandlerManager {
  /**
   * 初始化所有 IPC 处理器
   */
  static initialize(): void {
    logger.debug('[主进程] 开始注册 IPC 事件处理器')

    this.setupOverlayWindowHandlers()
    this.setupAutoStartHandlers()
    this.setupMainWindowHandlers()
    this.setupStoreHandlers()
    this.setupAnimationHandlers()
    this.setupScheduledSyncHandlers()
    this.setupAppRestartHandlers()

    logger.debug('[主进程] IPC 事件处理器注册完成')
  }

  /**
   * 设置 Overlay 窗口相关 IPC 处理器
   */
  private static setupOverlayWindowHandlers(): void {
    // 打开 overlay 窗口
    ipcMain.on('open-overlay-window', () => {
      try {
        if (overlayWindowManager.isValid()) {
          // 窗口已存在，切换显示状态
          overlayWindowManager.toggleWindow()
        } else {
          // 窗口不存在，创建新窗口并显示
          overlayWindowManager.createWindow()
          overlayWindowManager.showWindow()
        }
      } catch (error) {
        logger.error('[主进程] 打开 Overlay 窗口失败:', error)
      }
    })

    // 关闭 overlay 窗口
    ipcMain.on('close-overlay-window', () => {
      try {
        overlayWindowManager.hideWindow()
      } catch (error) {
        logger.error('[主进程] 关闭 Overlay 窗口失败:', error)
      }
    })
  }

  /**
   * 设置自启动相关 IPC 处理器
   */
  private static setupAutoStartHandlers(): void {
    // 获取自启动状态
    ipcMain.handle('auto-start-get-status', async () => {
      try {
        const isEnabled = await autoStartManager.isEnabled()
        const platformInfo = autoStartManager.getPlatformInfo()
        return {
          success: true,
          data: {
            enabled: isEnabled,
            platform: platformInfo.platform,
            method: platformInfo.method
          }
        }
      } catch (error) {
        logger.error('[主进程] 获取自启动状态失败:', error)
        return { success: false, error: (error as Error).message }
      }
    })

    // 设置自启动状态
    ipcMain.handle('auto-start-set-status', async (_, enabled: boolean) => {
      try {
        const success = await autoStartManager.setEnabled(enabled)
        return { success, data: { enabled } }
      } catch (error) {
        logger.error('[主进程] 设置自启动状态失败:', error)
        return { success: false, error: (error as Error).message }
      }
    })

    // 切换自启动状态
    ipcMain.handle('auto-start-toggle', async () => {
      try {
        const newState = await autoStartManager.toggle()
        return { success: true, data: { enabled: newState } }
      } catch (error) {
        logger.error('[主进程] 切换自启动状态失败:', error)
        return { success: false, error: (error as Error).message }
      }
    })
  }

  /**
   * 设置主窗口相关 IPC 处理器
   */
  private static setupMainWindowHandlers(): void {
    // 打开主窗口（设置页面）
    ipcMain.on('open-main-window', () => {
      try {
        mainWindowManager.showWindow()
      } catch (error) {
        logger.error('[主进程] 打开主窗口失败:', error)
      }
    })
  }

  /**
   * 设置存储相关 IPC 处理器
   */
  private static setupStoreHandlers(): void {
    // 获取存储值
    ipcMain.handle('store-get', (_, key: string, defaultValue?: unknown) => {
      try {
        return { success: true, data: store.get(key, defaultValue) }
      } catch (error) {
        logger.error(`[主进程] 获取存储值失败 [${key}]:`, error)
        return { success: false, error: (error as Error).message }
      }
    })

    // 同步获取存储值
    ipcMain.on('store-get-on', (event, key) => {
      try {
        const val = store?.get(key)
        event.returnValue = val
      } catch (error) {
        logger.error(`[主进程] 同步获取存储值失败 [${key}]:`, error)
        event.returnValue = undefined
      }
    })

    // 设置存储值
    ipcMain.handle('store-set', async (_, key: string, value: unknown) => {
      try {
        logger.debug(`[主进程] 设置存储值 [${key}]:`, value)
        store.raw_set(key, value)

        // 处理特殊配置更新
        await this.handleSpecialConfigUpdates(key, value)

        return { success: true }
      } catch (error) {
        logger.error(`[主进程] 设置存储值失败 [${key}]:`, error)
        return { success: false, error: (error as Error).message }
      }
    })
  }

  /**
   * 设置动画相关 IPC 处理器
   */
  private static setupAnimationHandlers(): void {
    // 监听前端动画完成消息
    ipcMain.on('open-overlay-finished', () => {
      if (windowStateManager.getOverlayState() === OverlayState.OPENING) {
        windowStateManager.setOverlayState(OverlayState.OPENED)
      }
    })
  }

  /**
   * 处理特殊配置更新
   */
  private static async handleSpecialConfigUpdates(key: string, value: unknown): Promise<void> {
    try {
      // 如果更新的是全局快捷键配置，重新初始化快捷键
      if (key === STORE_KEY.SETTINGS_GLOBAL_SHORTCUT) {
        logger.debug('[主进程] 全局快捷键配置已更新，重新初始化')

        // 先取消所有已注册的快捷键
        globalShortcutManager.unregisterAll()

        // 重新注册快捷键
        const shortcutKey = value as string
        if (shortcutKey && shortcutKey.trim() !== '') {
          globalShortcutManager.registerGlobalShortcut(shortcutKey, () => {
            overlayWindowManager.toggleWindow(true)
          })
        }
      }

      // 如果更新的是最大历史记录数量配置，使用防抖更新剪贴板管理器配置
      if (key === STORE_KEY.SETTINGS_MAX_HISTORY_SIZE) {
        debounceExecution(
          'update-max-history-size',
          async () => {
            logger.debug('[主进程] 最大历史记录数量配置已更新，更新剪贴板管理器配置')
            try {
              const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
              await clipboardManager.updateConfig({ maxHistorySize })
            } catch (error) {
              logger.error('[主进程] 更新剪贴板管理器maxHistorySize失败:', error)
            }
          },
          MAX_HISTORY_SIZE_DEBOUNCE_DELAY
        )
      }

      // 如果更新的是最大保存时间配置，使用防抖更新历史记录清理器配置
      if (key === STORE_KEY.SETTINGS_MAX_KEEP_TIME) {
        debounceExecution(
          'update-max-keep-time',
          async () => {
            logger.debug('[主进程] 最大保存时间配置已更新，更新清理器配置')
            try {
              const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1)
              await clipboardHistoryCleaner.call('updateConfig', {
                maxKeepTime
              })
            } catch (error) {
              logger.error('[主进程] 更新清理器配置失败:', error)
            }
          },
          MAX_KEEP_TIME_DEBOUNCE_DELAY
        )
      }

      // 如果更新的是自动启动配置，设置应用的自动启动状态
      if (key === STORE_KEY.SETTINGS_AUTO_START) {
        const autoStart = value as boolean
        const success = await autoStartManager.setEnabled(autoStart)

        if (!success) {
          logger.error('[主进程] 设置自动启动失败')
        } else {
          logger.debug(`[主进程] 自动启动状态已更新: ${autoStart}`)
        }
      }

      // 如果更新的是本地OCR配置，更新剪贴板管理器配置
      if (key === STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR) {
        const enableLocalOCR = value as boolean
        logger.debug(`[主进程] 本地OCR配置已更新: ${enableLocalOCR}`)

        try {
          await clipboardManager.updateConfig({ enableLocalOCR })
          logger.debug('[主进程] 剪贴板管理器本地OCR配置已更新')
        } catch (error) {
          logger.error('[主进程] 更新剪贴板管理器本地OCR配置失败:', error)
        }
      }

      // 如果更新的是同步启用状态，根据值开启或关闭同步功能
      // if (key === STORE_KEY.SYNC_ENABLED) {
      //   const syncEnabled = value as boolean
      //   logger.debug(`[主进程] 同步启用状态已更新: ${syncEnabled}`)

      //   if (syncEnabled) {
      //     // 启用同步：重新加载同步配置并初始化定时同步
      //     try {
      //       const syncConfig = SyncConfigUtils.loadSyncConfig()

      //       if (syncConfig && syncConfig.enabled) {
      //         logger.info('[主进程] 同步已启用，开始初始化定时同步')

      //         // 初始化定时同步管理器
      //         const initialized = await scheduledSyncManager.initialize(syncConfig)

      //         if (initialized) {
      //           logger.info('[主进程] 定时同步服务初始化并启动成功')
      //         } else {
      //           logger.warn('[主进程] 定时同步服务初始化失败')
      //         }
      //       } else {
      //         logger.warn('[主进程] 同步配置不完整，无法启动定时同步')
      //       }
      //     } catch (error) {
      //       logger.error('[主进程] 启用同步失败:', error)
      //     }
      //   } else {
      //     // 禁用同步：停止定时同步
      //     try {
      //       logger.info('[主进程] 同步已禁用，停止定时同步')

      //       const stopped = await scheduledSyncManager.stopScheduledSync()

      //       if (stopped) {
      //         logger.info('[主进程] 定时同步已成功停止')
      //       } else {
      //         logger.warn('[主进程] 停止定时同步失败')
      //       }
      //     } catch (error) {
      //       logger.error('[主进程] 禁用同步失败:', error)
      //     }
      //   }
      // }
      logger.debug(`[主进程] 处理特殊配置更新完成 [${key}]`)
    } catch (error) {
      logger.error(`[主进程] 处理特殊配置更新失败 [${key}]:`, error)
    }
  }

  /**
   * 设置定时同步相关 IPC 处理器
   */
  private static setupScheduledSyncHandlers(): void {
    // 初始化定时同步
    ipcMain.handle(
      'scheduled-sync:initialize',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        try {
          const syncConfig: ScheduledSyncConfig = {
            ...config,
            userDataPath: app.getPath('userData')
          }
          return await scheduledSyncManager.initialize(syncConfig)
        } catch (error) {
          logger.error('[主进程] 初始化定时同步失败:', error)
          return false
        }
      }
    )

    // 启动定时同步
    ipcMain.handle('scheduled-sync:start', async () => {
      try {
        return await scheduledSyncManager.startScheduledSync()
      } catch (error) {
        logger.error('[主进程] 启动定时同步失败:', error)
        return false
      }
    })

    // 停止定时同步
    ipcMain.handle('scheduled-sync:stop', async () => {
      try {
        return await scheduledSyncManager.stopScheduledSync()
      } catch (error) {
        logger.error('[主进程] 停止定时同步失败:', error)
        return false
      }
    })

    // 手动同步
    ipcMain.handle('scheduled-sync:manual-sync', async () => {
      try {
        return await scheduledSyncManager.manualSync()
      } catch (error) {
        logger.error('[主进程] 手动同步失败:', error)
        return {
          success: false,
          message: error instanceof Error ? error.message : '未知错误',
          errors: [error instanceof Error ? error.message : '未知错误']
        }
      }
    })

    // 使用指定配置手动同步
    ipcMain.handle(
      'scheduled-sync:manual-sync-with-config',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        try {
          const syncConfig: ScheduledSyncConfig = {
            ...config,
            userDataPath: app.getPath('userData')
          }
          return await scheduledSyncManager.manualSyncWithConfig(syncConfig)
        } catch (error) {
          logger.error('[主进程] 使用指定配置手动同步失败:', error)
          return {
            success: false,
            message: error instanceof Error ? error.message : '未知错误',
            errors: [error instanceof Error ? error.message : '未知错误']
          }
        }
      }
    )

    // 测试连接
    ipcMain.handle(
      'scheduled-sync:test-connection',
      async (_, config?: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        try {
          if (config) {
            // 如果传入了配置，使用传入的配置进行测试
            const syncConfig: ScheduledSyncConfig = {
              ...config,
              userDataPath: app.getPath('userData')
            }
            return await scheduledSyncManager.testConnectionWithConfig(syncConfig)
          } else {
            // 如果没有传入配置，使用当前配置进行测试
            return await scheduledSyncManager.testConnection()
          }
        } catch (error) {
          logger.error('[主进程] 测试同步连接失败:', error)
          return false
        }
      }
    )

    // 获取同步状态
    ipcMain.handle('scheduled-sync:get-status', async () => {
      try {
        // 检查同步管理器是否正在运行
        if (!scheduledSyncManager.isRunning()) {
          // 如果同步功能未开启，返回默认状态
          return {
            success: true,
            data: {
              isRunning: false,
              lastSyncTime: 0,
              nextSyncTime: 0,
              syncStatus: SyncStatus.IDLE,
              config: null
            }
          }
        }

        const status = await scheduledSyncManager.getSyncStatus()
        return { success: true, data: status }
      } catch (error) {
        logger.error('[主进程] 获取同步状态失败:', error)
        return { success: false, error: (error as Error).message }
      }
    })

    // 更新配置
    ipcMain.handle(
      'scheduled-sync:update-config',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        try {
          const syncConfig: ScheduledSyncConfig = {
            ...config,
            userDataPath: app.getPath('userData')
          }
          return await scheduledSyncManager.updateConfig(syncConfig)
        } catch (error) {
          logger.error('[主进程] 更新定时同步配置失败:', error)
          return false
        }
      }
    )

    // 刷新计划同步任务
    ipcMain.handle('scheduled-sync:refresh', async () => {
      try {
        logger.info('[主进程] 开始刷新计划同步任务')

        // 从存储中读取基础同步配置（即使 provider 不可用也能获取配置）
        const syncConfig = SyncConfigUtils.loadBaseSyncConfig()

        logger.debug('[主进程] 已加载基础同步配置:', {
          enabled: syncConfig.enabled,
          provider: syncConfig.providerType,
          interval: syncConfig.syncInterval
        })

        // 使用 updateConfig 方法来重启计划同步任务
        // 该方法会自动处理停止、重新初始化和根据 enabled 状态启动的逻辑
        const result = await scheduledSyncManager.updateConfig(syncConfig)

        if (result) {
          if (syncConfig.enabled) {
            logger.info('[主进程] 计划同步任务刷新成功，同步已启用')
          } else {
            logger.info('[主进程] 计划同步任务刷新成功，Worker 基础功能可用')
          }
        } else {
          logger.warn('[主进程] 计划同步任务刷新失败')
        }

        return result
      } catch (error) {
        logger.error('[主进程] 刷新计划同步任务失败:', error)
        return false
      }
    })
  }

  /**
   * 设置应用重启相关 IPC 处理器
   */
  private static setupAppRestartHandlers(): void {
    // 重启应用
    ipcMain.handle('app:restart', async () => {
      try {
        logger.info('[主进程] 收到应用重启请求')

        // 延迟重启，给前端时间处理通知
        setTimeout(() => {
          app.relaunch()
          app.exit()
        }, 1000)

        return { success: true }
      } catch (error) {
        logger.error('[主进程] 应用重启失败:', error)
        return { success: false, error: (error as Error).message }
      }
    })
  }

  /**
   * 清理所有 IPC 处理器
   */
  static cleanup(): void {
    // IPC 处理器会随着 app 退出自动清理
    logger.debug('[主进程] IPC 事件处理器已清理')
  }
}
