import { app } from 'electron'
import { mainWindowManager } from '../ui/windows/window-managers'
import logger from '../utils/logger'
import { appEventBus } from '../utils/app-event-bus'

class AppLifecycleManager {
  private static instance: AppLifecycleManager
  private isQuitting: boolean = false

  private constructor() {
    // 通过静态方法统一创建单例。
  }

  public static getInstance(): AppLifecycleManager {
    if (!AppLifecycleManager.instance) {
      AppLifecycleManager.instance = new AppLifecycleManager()
    }
    return AppLifecycleManager.instance
  }

  /**
   * Hide the dock icon (macOS only)
   */
  public hideDock(): void {
    if (process.platform === 'darwin') {
      app.dock?.hide()
    }
  }

  /**
   * Show the dock icon (macOS only)
   */
  public showDock(): void {
    if (process.platform === 'darwin') {
      app.dock?.show()
    }
  }

  /**
   * Handle window all closed event
   */
  public handleWindowAllClosed(): void {
    logger.debug('[AppLifecycle] All windows closed')
    // On macOS, applications typically stay open until the user quits explicitly
    // Since we have a tray, we generally stay open on other platforms too
  }

  /**
   * Handle before-quit event
   */
  public handleBeforeQuit(): void {
    this.isQuitting = true
    logger.info('[AppLifecycle] Before quit event received')
  }

  public restart(): void {
    if (this.isQuitting) return

    app.relaunch({
      // AppImage 退出后会卸载内层可执行文件所在的临时目录。
      ...(process.platform === 'linux' && app.isPackaged && process.env.APPIMAGE
        ? { execPath: process.env.APPIMAGE }
        : {})
    })
    this.isQuitting = true
    // 先返回可能的 IPC 响应，再通过统一退出流程保存数据并释放资源。
    setTimeout(() => appEventBus.requestAppQuit(), 0)
  }

  /**
   * Handle application activation (e.g. clicking dock icon)
   */
  public handleActivate(isInitialized: boolean): void {
    if (!isInitialized) return
    mainWindowManager.showWindow()
  }

  public getIsQuitting(): boolean {
    return this.isQuitting
  }
}

export const appLifecycleManager = AppLifecycleManager.getInstance()
