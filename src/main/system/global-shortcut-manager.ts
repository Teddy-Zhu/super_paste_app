import { globalShortcut } from 'electron'
import { WindowUtils, windowStateManager } from '../ui/windows/window-state'
import logger from '../utils/logger'

/**
 * 全局快捷键管理器
 */
export class GlobalShortcutManager {
  private overlayWindow: Electron.BrowserWindow | null = null

  /**
   * 设置 overlay 窗口引用
   * @param window overlay 窗口实例
   */
  setOverlayWindow(window: Electron.BrowserWindow | null): void {
    this.overlayWindow = window
  }

  /**
   * 注册 ESC 键监听
   */
  registerEscapeKey(): void {
    if (!globalShortcut.isRegistered('Escape')) {
      globalShortcut.register('Escape', () => {
        if (
          WindowUtils.isWindowValidAndVisible(this.overlayWindow) &&
          windowStateManager.canCloseOverlay()
        ) {
          WindowUtils.safelyEmitToWindow(this.overlayWindow, 'trigger-close-window')
        }
      })
      logger.debug('[主进程] ESC 键监听已注册')
    }
  }

  /**
   * 注销 ESC 键监听
   */
  unregisterEscapeKey(): void {
    if (globalShortcut.isRegistered('Escape')) {
      globalShortcut.unregister('Escape')
      logger.debug('[主进程] ESC 键监听已注销')
    }
  }

  /**
   * 注册全局快捷键
   * @param shortcutKey 快捷键字符串
   * @param callback 回调函数
   */
  registerGlobalShortcut(shortcutKey: string, callback: () => void): boolean {
    if (!shortcutKey || shortcutKey.trim() === '') {
      logger.debug('[主进程] 未配置全局快捷键，跳过注册')
      return false
    }

    const success = globalShortcut.register(shortcutKey, callback)

    if (success) {
      logger.debug(`[主进程] 全局快捷键注册成功: ${shortcutKey}`)
    } else {
      logger.error(`[主进程] 全局快捷键注册失败: ${shortcutKey}`)
    }

    return success
  }

  /**
   * 注销指定快捷键
   * @param shortcutKey 快捷键字符串
   */
  unregisterShortcut(shortcutKey: string): void {
    if (globalShortcut.isRegistered(shortcutKey)) {
      globalShortcut.unregister(shortcutKey)
      logger.debug(`[主进程] 快捷键已注销: ${shortcutKey}`)
    }
  }

  /**
   * 注销所有快捷键
   */
  unregisterAll(): void {
    globalShortcut.unregisterAll()
    logger.debug('[主进程] 所有全局快捷键已注销')
  }

  /**
   * 检查快捷键是否已注册
   * @param shortcutKey 快捷键字符串
   */
  isRegistered(shortcutKey: string): boolean {
    return globalShortcut.isRegistered(shortcutKey)
  }
}

// 导出单例实例
export const globalShortcutManager = new GlobalShortcutManager()
