import { ipcMain } from 'electron'
import { mainWindowManager, overlayWindowManager } from '../ui/windows/window-managers'
import { windowStateManager, OverlayState } from '../ui/windows/window-state'
import { globalShortcutManager } from '../system/global-shortcut-manager'
import autoStartManager from '../utils/auto-start-manager'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import { debounceExecution } from '../utils/common'
import { MAX_HISTORY_SIZE_DEBOUNCE_DELAY, MAX_KEEP_TIME_DEBOUNCE_DELAY } from '../config/constants'
import clipboardManager from '../clipboard/clipboard-manager'
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
          () => {
            clipboardManager.updateMaxHistorySizeFromStore()
          },
          MAX_HISTORY_SIZE_DEBOUNCE_DELAY
        )
      }

      // 如果更新的是最大保存时间配置，使用防抖更新历史记录清理器配置
      if (key === STORE_KEY.SETTINGS_MAX_KEEP_TIME) {
        debounceExecution(
          'update-max-keep-time',
          () => {
            clipboardManager.updateMaxKeepTimeFromStore()
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
    } catch (error) {
      logger.error(`[主进程] 处理特殊配置更新失败 [${key}]:`, error)
    }
  }

  /**
   * 清理所有 IPC 处理器
   */
  static cleanup(): void {
    // IPC 处理器会随着 app 退出自动清理
    logger.debug('[主进程] IPC 事件处理器已清理')
  }
}
