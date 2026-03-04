import { globalShortcut } from 'electron'
import { WindowUtils, windowStateManager } from '../ui/windows/window-state'
import logger from '../utils/logger'

/**
 * 全局快捷键管理器
 */
export class GlobalShortcutManager {
  private overlayWindow: Electron.BrowserWindow | null = null
  private currentGlobalShortcut: string | null = null

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

    // 如果更新为新的主快捷键，先清理旧快捷键，避免残留占用
    if (
      this.currentGlobalShortcut &&
      this.currentGlobalShortcut !== shortcutKey &&
      globalShortcut.isRegistered(this.currentGlobalShortcut)
    ) {
      globalShortcut.unregister(this.currentGlobalShortcut)
      logger.debug(`[主进程] 旧全局快捷键已注销: ${this.currentGlobalShortcut}`)
    }

    // 如果已经注册过，先注销
    if (globalShortcut.isRegistered(shortcutKey)) {
      globalShortcut.unregister(shortcutKey)
      logger.debug(`[主进程] 已存在相同快捷键，已注销: ${shortcutKey}`)
    }

    const success = globalShortcut.register(shortcutKey, callback)

    if (success) {
      this.currentGlobalShortcut = shortcutKey
      logger.debug(`[主进程] 全局快捷键注册成功: ${shortcutKey}`)
    } else {
      if (this.currentGlobalShortcut === shortcutKey) {
        this.currentGlobalShortcut = null
      }
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

    if (this.currentGlobalShortcut === shortcutKey) {
      this.currentGlobalShortcut = null
    }
  }

  /**
   * 注销所有快捷键
   */
  unregisterAll(): void {
    globalShortcut.unregisterAll()
    this.currentGlobalShortcut = null
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
