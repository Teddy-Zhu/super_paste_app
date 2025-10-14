import { BrowserWindow, shell, app } from 'electron'
import { join } from 'path'
import { is } from '@electron-toolkit/utils'
import { WindowConfigCalculator, IconSelector } from './window-config'
import { WindowUtils, windowStateManager, OverlayState } from './window-state'
import { globalShortcutManager } from '../../system/global-shortcut-manager'
import store from '../../config/store'
import { STORE_KEY } from '../../config/common'
import logger from '../../utils/logger'

/**
 * 主窗口管理器
 */
export class MainWindowManager {
  private mainWindow: BrowserWindow | null = null

  /**
   * 获取主窗口实例
   */
  getWindow(): BrowserWindow | null {
    return this.mainWindow
  }

  /**
   * 创建主窗口
   */
  createWindow(): void {
    try {
      logger.debug('[主进程] 开始创建主窗口')

      // 计算最佳窗口尺寸
      const { width, height } = WindowConfigCalculator.calculateMainWindowSize()

      // 创建浏览器窗口
      this.mainWindow = new BrowserWindow({
        width,
        height,
        minWidth: width,
        minHeight: height,
        maxWidth: width,
        maxHeight: height,
        show: false, // 确保窗口创建时不显示
        center: true, // 窗口居中显示
        hasShadow: false,
        autoHideMenuBar: true,
        resizable: false, // 禁止调整大小
        skipTaskbar: true, // 在 Windows 和 Linux 上不显示在任务栏
        ...(process.platform === 'linux' ? { icon: IconSelector.getWindowIcon() } : {}),
        webPreferences: {
          preload: join(__dirname, '../preload/index.js'),
          sandbox: false,
          nodeIntegration: false,
          contextIsolation: true
        }
      })

      this.setupEventHandlers()
      this.loadContent()

      // 在 macOS 上创建窗口后立即隐藏 dock
      if (process.platform === 'darwin') {
        app.dock?.hide()
      }

      logger.debug('[主进程] 主窗口创建完成')
    } catch (error) {
      logger.error('[主进程] 创建主窗口时发生错误:', error)
      throw error
    }
  }

  /**
   * 设置事件处理器
   */
  private setupEventHandlers(): void {
    if (!this.mainWindow) return

    this.mainWindow.on('ready-to-show', () => {
      logger.debug('[主进程] 主窗口准备就绪')
      if (is.dev) {
        this.mainWindow?.webContents.openDevTools()
      }
    })

    // 监听主窗口显示事件
    this.mainWindow.on('show', () => {
      // 仅在 macOS 上控制 dock 显示
      if (process.platform === 'darwin') {
        app.dock?.show()
      }
    })

    // 监听主窗口隐藏事件
    this.mainWindow.on('hide', () => {
      // 仅在 macOS 上控制 dock 隐藏
      if (process.platform === 'darwin') {
        app.dock?.hide()
      }
    })

    // 监听主窗口关闭事件
    this.mainWindow.on('closed', () => {
      this.mainWindow = null
      // 仅在 macOS 上控制 dock 隐藏
      if (process.platform === 'darwin') {
        app.dock?.hide()
      }
    })

    this.mainWindow.webContents.setWindowOpenHandler((details) => {
      shell.openExternal(details.url)
      return { action: 'deny' }
    })
  }

  /**
   * 加载窗口内容
   */
  private loadContent(): void {
    if (!this.mainWindow) return

    // HMR for renderer base on electron-vite cli.
    // Load the remote URL for development or the local html file for production.
    if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
      this.mainWindow
        .loadURL(process.env['ELECTRON_RENDERER_URL'])
        .then(() => {
          logger.debug('[主进程] 主窗口加载完成 (开发模式)')
        })
        .catch((error) => {
          logger.error('[主进程] 主窗口加载失败 (开发模式):', error)
        })
    } else {
      this.mainWindow
        .loadFile(join(__dirname, '../renderer/index.html'))
        .then(() => {
          logger.debug('[主进程] 主窗口加载完成 (生产模式)')
        })
        .catch((error) => {
          logger.error('[主进程] 主窗口加载失败 (生产模式):', error)
        })
    }
  }

  /**
   * 显示主窗口（增强版，带错误处理）
   */
  showWindow(): void {
    try {
      if (!this.mainWindow || this.mainWindow.isDestroyed()) {
        logger.debug('[主进程] 主窗口不存在或已销毁，创建新窗口')
        this.createWindow()
        return
      }

      // 如果窗口存在但被隐藏，则显示它
      WindowUtils.safelyShowWindow(this.mainWindow)

      // 确保窗口获得焦点
      WindowUtils.safelyFocusWindow(this.mainWindow)
    } catch (error) {
      logger.error('[主进程] 显示主窗口时发生错误:', error)
      // 尝试重新创建窗口
      try {
        this.createWindow()
      } catch (createError) {
        logger.error('[主进程] 重新创建主窗口失败:', createError)
      }
    }
  }

  /**
   * 销毁主窗口
   */
  destroy(): void {
    WindowUtils.safelyDestroyWindow(this.mainWindow)
    this.mainWindow = null
  }

  /**
   * 检查窗口是否存在且未销毁
   */
  isValid(): boolean {
    return WindowUtils.isWindowValid(this.mainWindow)
  }

  /**
   * 检查窗口是否可见
   */
  isVisible(): boolean {
    return WindowUtils.isWindowValidAndVisible(this.mainWindow)
  }

  /**
   * 检查窗口是否最小化
   */
  isMinimized(): boolean {
    return this.mainWindow ? this.mainWindow.isMinimized() : false
  }

  /**
   * 恢复窗口
   */
  restore(): void {
    if (this.mainWindow && this.mainWindow.isMinimized()) {
      this.mainWindow.restore()
    }
  }
}

/**
 * Overlay 窗口管理器
 */
export class OverlayWindowManager {
  private static readonly SLIDE_IN_DELAY_MS = 16
  private overlayWindow: BrowserWindow | null = null
  private slideInTimer: NodeJS.Timeout | null = null

  /**
   * 获取 overlay 窗口实例
   */
  getWindow(): BrowserWindow | null {
    return this.overlayWindow
  }

  /**
   * 创建覆盖层窗口
   */
  createWindow(): void {
    try {
      if (this.overlayWindow) {
        return
      }

      logger.debug('[主进程] 开始创建 Overlay 窗口')

      // 计算窗口位置
      const position = WindowConfigCalculator.calculateWindowPosition()

      // 创建覆盖层窗口
      this.overlayWindow = new BrowserWindow({
        width: position.width,
        height: position.height,
        x: position.x,
        y: position.y,
        frame: false,
        alwaysOnTop: true,
        skipTaskbar: true,
        resizable: false,
        movable: false,
        minimizable: false,
        maximizable: false,
        closable: true,
        roundedCorners: false,
        focusable: true,
        show: false,
        type: 'utility',
        transparent: true,
        paintWhenInitiallyHidden: true,
        ...(process.platform === 'linux' ? { icon: IconSelector.getWindowIcon() } : {}),
        webPreferences: {
          preload: join(__dirname, '../preload/index.js'),
          sandbox: false,
          nodeIntegration: false,
          contextIsolation: true,
          backgroundThrottling: false
        }
      })

      // 设置快捷键管理器的窗口引用
      globalShortcutManager.setOverlayWindow(this.overlayWindow)

      this.resetOverlayBounds()

      this.setupEventHandlers()
      this.loadContent()

      logger.debug('[主进程] Overlay 窗口创建完成')
    } catch (error) {
      logger.error('[主进程] 创建 Overlay 窗口时发生错误:', error)
      this.overlayWindow = null
      throw error
    }
  }

  /**
   * 设置事件处理器
   */
  private setupEventHandlers(): void {
    if (!this.overlayWindow) return

    this.overlayWindow.on('ready-to-show', () => {
      if (is.dev) {
        this.overlayWindow?.webContents.openDevTools()
      }

      // 在窗口准备就绪后初始化全局快捷键（最后执行）
      logger.debug('[主进程] Overlay 窗口准备就绪，开始初始化全局快捷键')
      this.initializeGlobalShortcut()
      logger.debug('[主进程] 全局快捷键初始化完成，应用启动流程结束')
    })

    this.overlayWindow.on('show', () => {
      this.scheduleSlideInAnimation()
    })

    this.overlayWindow.on('focus', () => {
      // 窗口已在前台显示并获得焦点
    })

    // 阻止窗口关闭，改为隐藏
    this.overlayWindow.on('close', (event) => {
      event.preventDefault()
      this.hideWindow()
    })

    this.overlayWindow.on('closed', () => {
      this.clearSlideInTimer()
      // 取消全局 ESC 键监听
      globalShortcutManager.unregisterEscapeKey()

      // 重置overlay状态
      windowStateManager.setOverlayState(OverlayState.CLOSED)

      this.overlayWindow = null
    })

    // 失去焦点时隐藏窗口
    this.overlayWindow.on('blur', () => {
      if (this.overlayWindow && !is.dev) {
        this.hideWindowBackend(true)
      }
    })

    this.overlayWindow.setSkipTaskbar(true)
  }

  /**
   * 加载窗口内容
   */
  private loadContent(): void {
    if (!this.overlayWindow) return

    // 加载覆盖层页面
    if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
      const url = `${process.env['ELECTRON_RENDERER_URL']}#/overlay`
      this.overlayWindow.loadURL(url)
    } else {
      this.overlayWindow.loadFile(join(__dirname, '../renderer/index.html'), { hash: '/overlay' })
    }
  }

  /**
   * 在窗口处于隐藏状态时归一化高度，避免反复切换导致窗口逐步增高
   */
  private resetOverlayBounds(): void {
    const windowInstance = this.overlayWindow

    if (!windowInstance || windowInstance.isDestroyed() || windowInstance.isVisible()) {
      return
    }

    try {
      const currentBounds = windowInstance.getBounds()
      const targetHeight = WindowConfigCalculator.calculateWindowPosition().height
      console.log('resetOverlayBounds', currentBounds, targetHeight)
      if (currentBounds.height === targetHeight) {
        return
      }

      const bottom = currentBounds.y + currentBounds.height
      windowInstance.setBounds({
        ...currentBounds,
        height: targetHeight,
        y: bottom - targetHeight
      })
    } catch (error) {
      logger.error('[主进程] 重置 Overlay 窗口位置失败:', error)
    }
  }

  /**
   * 延迟触发滑入动画，确保窗口已经渲染完首帧，避免动画在不可见状态下执行
   */
  private scheduleSlideInAnimation(): void {
    if (windowStateManager.getOverlayState() !== OverlayState.OPENING) {
      return
    }

    if (!WindowUtils.isWindowValidAndVisible(this.overlayWindow)) {
      return
    }

    this.clearSlideInTimer()

    this.slideInTimer = setTimeout(() => {
      this.slideInTimer = null
      WindowUtils.safelyEmitToWindow(this.overlayWindow, 'trigger-slide-in-animation')
    }, OverlayWindowManager.SLIDE_IN_DELAY_MS)
  }

  private clearSlideInTimer(): void {
    if (this.slideInTimer) {
      clearTimeout(this.slideInTimer)
      this.slideInTimer = null
    }
  }

  /**
   * 初始化全局快捷键
   */
  private initializeGlobalShortcut(): void {
    try {
      // 从store读取全局快捷键配置
      const shortcutKey = store.get(STORE_KEY.SETTINGS_GLOBAL_SHORTCUT, '') as string

      if (shortcutKey && shortcutKey.trim() !== '') {
        globalShortcutManager.registerGlobalShortcut(shortcutKey, () => {
          this.toggleWindow(true)
        })
      } else {
        logger.debug('[主进程] 未配置全局快捷键，跳过注册')
      }
    } catch (error) {
      logger.error('[主进程] 初始化全局快捷键时发生错误:', error)
    }
  }

  /**
   * 显示 overlay 窗口
   */
  showWindow(): void {
    if (!this.overlayWindow) {
      return
    }

    // 检查是否可以执行打开操作
    if (!windowStateManager.canOpenOverlay()) {
      return
    }

    // 设置状态为正在打开
    windowStateManager.setOverlayState(OverlayState.OPENING)

    // 显示并聚焦窗口
    WindowUtils.safelyShowWindow(this.overlayWindow)
    WindowUtils.safelyFocusWindow(this.overlayWindow)

    // 注册全局 ESC 键监听
    globalShortcutManager.registerEscapeKey()
  }

  /**
   * 隐藏窗口（后端触发）
   */
  hideWindowBackend(force = false): void {
    if (!this.overlayWindow) {
      return
    }

    // 检查是否可以执行关闭操作
    if (!windowStateManager.canCloseOverlay()) {
      return
    }

    // 设置状态为正在关闭
    windowStateManager.setOverlayState(OverlayState.CLOSING)

    // 发送关闭信号到前端
    WindowUtils.safelyEmitToWindow(this.overlayWindow, 'trigger-close-window', force)
  }

  /**
   * 隐藏 overlay 窗口
   */
  hideWindow(): void {
    if (!this.overlayWindow) {
      return
    }

    if (!this.overlayWindow.isVisible()) {
      // 确保状态正确
      windowStateManager.setOverlayState(OverlayState.CLOSED)
      return
    }

    // 隐藏窗口
    WindowUtils.safelyHideWindow(this.overlayWindow)

    this.clearSlideInTimer()
    // this.resetOverlayBounds()

    // 取消全局 ESC 键监听
    globalShortcutManager.unregisterEscapeKey()

    // 设置状态为已关闭
    windowStateManager.setOverlayState(OverlayState.CLOSED)
  }

  /**
   * 切换 overlay 窗口显示状态
   */
  toggleWindow(force = false): void {
    if (!this.overlayWindow) {
      return
    }

    const currentState = windowStateManager.getOverlayState()

    // 根据当前状态决定操作
    switch (currentState) {
      case OverlayState.CLOSED:
        this.showWindow()
        break

      case OverlayState.OPENED:
        this.hideWindowBackend(force)
        break

      case OverlayState.OPENING:
      case OverlayState.CLOSING:
        // 忽略切换请求
        break

      default:
        logger.warn(`[主进程] 未知的overlay状态: ${currentState}`)
        break
    }
  }

  /**
   * 销毁 overlay 窗口
   */
  destroy(): void {
    this.clearSlideInTimer()
    WindowUtils.safelyDestroyWindow(this.overlayWindow)
    this.overlayWindow = null
  }

  /**
   * 检查窗口是否存在且未销毁
   */
  isValid(): boolean {
    return WindowUtils.isWindowValid(this.overlayWindow)
  }

  /**
   * 检查窗口是否可见
   */
  isVisible(): boolean {
    return WindowUtils.isWindowValidAndVisible(this.overlayWindow)
  }
}

// 导出单例实例
export const mainWindowManager = new MainWindowManager()
export const overlayWindowManager = new OverlayWindowManager()
