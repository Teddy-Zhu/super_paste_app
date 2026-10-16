import { BrowserWindow, shell } from 'electron'
import { join } from 'path'
import { is } from '@electron-toolkit/utils'
import {
  WindowConfigCalculator,
  IconSelector,
  DEFAULT_OVERLAY_POSITION,
  type OverlayPosition
} from './window-config'
import { WindowUtils, windowStateManager, OverlayState } from './window-state'
import { globalShortcutManager } from '../../system/global-shortcut-manager'
import { appLifecycleManager } from '../../system/app-lifecycle-manager'
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
        frame: false,
        show: false, // 确保窗口创建时不显示
        center: true, // 窗口居中显示
        hasShadow: false,
        backgroundColor: '#eef2f7',
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
      appLifecycleManager.hideDock()

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
      appLifecycleManager.showDock()
    })

    // 监听主窗口隐藏事件
    this.mainWindow.on('hide', () => {
      // 仅在 macOS 上控制 dock 隐藏
      appLifecycleManager.hideDock()
    })

    // 监听主窗口关闭事件
    this.mainWindow.on('closed', () => {
      this.mainWindow = null
      // 仅在 macOS 上控制 dock 隐藏
      appLifecycleManager.hideDock()
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
  private overlayWindow: BrowserWindow | null = null
  private rendererReady = false
  private wantsVisible = false
  private transitionId = 0
  private transitionTimer: NodeJS.Timeout | null = null
  private currentOverlayPosition: OverlayPosition = DEFAULT_OVERLAY_POSITION

  getWindow(): BrowserWindow | null {
    return this.overlayWindow
  }

  createWindow(): void {
    if (this.isValid()) return
    const position = this.getOverlayPositionSetting()
    const bounds = WindowConfigCalculator.calculateWindowPosition(position)
    this.overlayWindow = new BrowserWindow({
      ...bounds,
      frame: false,
      alwaysOnTop: true,
      skipTaskbar: true,
      resizable: false,
      movable: false,
      minimizable: false,
      maximizable: false,
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
    const window = this.overlayWindow
    window.once('ready-to-show', () => this.registerGlobalShortcut())
    window.on('close', (event) => {
      event.preventDefault()
      this.hideWindow()
    })
    window.on('closed', () => {
      this.clearTransitionTimer()
      this.rendererReady = false
      this.wantsVisible = false
      windowStateManager.reset()
      this.overlayWindow = null
    })
    window.on('blur', () => {
      if (!is.dev) this.hideWindowBackend(true)
    })
    window.webContents.on('did-start-loading', () => {
      const pendingShow = this.wantsVisible
      this.rendererReady = false
      this.hideWindow()
      this.wantsVisible = pendingShow
    })
    window.webContents.on('render-process-gone', () => {
      this.rendererReady = false
      this.hideWindow()
    })
    const loading =
      is.dev && process.env['ELECTRON_RENDERER_URL']
        ? window.loadURL(`${process.env['ELECTRON_RENDERER_URL']}#/overlay`)
        : window.loadFile(join(__dirname, '../renderer/index.html'), { hash: '/overlay' })
    loading.catch((error) => logger.error('[Overlay] 页面加载失败:', error))
  }

  markRendererReady(): void {
    this.rendererReady = true
    if (this.wantsVisible) this.showWindow()
  }

  private getOverlayPositionSetting(): OverlayPosition {
    return WindowConfigCalculator.normalizeOverlayPosition(
      store.get(STORE_KEY.SETTINGS_OVERLAY_POSITION, DEFAULT_OVERLAY_POSITION)
    )
  }

  registerGlobalShortcut(retry = false): boolean {
    return globalShortcutManager.registerGlobalShortcut(
      store.get<string>(STORE_KEY.SETTINGS_GLOBAL_SHORTCUT, ''),
      () => this.toggleWindow(true),
      retry
    )
  }

  showWindow(): void {
    if (!this.isValid()) this.createWindow()
    this.wantsVisible = true
    if (!this.rendererReady || !this.overlayWindow) return
    const state = windowStateManager.getOverlayState()
    if (state === OverlayState.OPENING || state === OverlayState.OPENED) return

    this.clearTransitionTimer()
    const id = ++this.transitionId
    this.currentOverlayPosition = this.getOverlayPositionSetting()
    try {
      const bounds = WindowConfigCalculator.calculateWindowPosition(this.currentOverlayPosition)
      const previous = this.overlayWindow.getBounds()
      if ((['x', 'y', 'width', 'height'] as const).some((key) => bounds[key] !== previous[key])) {
        this.overlayWindow.setBounds(bounds)
      }
      windowStateManager.setOverlayState(OverlayState.OPENING)
      this.overlayWindow.show()
      this.overlayWindow.focus()
      WindowUtils.safelyEmitToWindow(
        this.overlayWindow,
        'trigger-slide-in-animation',
        this.currentOverlayPosition,
        id
      )
      // 渲染进程停顿时也允许下一次快捷键关闭窗口。
      this.transitionTimer = setTimeout(() => this.finishOpening(id), 250)
    } catch (error) {
      logger.error('[Overlay] 显示失败:', error)
      this.hideWindow(id)
    }
  }

  finishOpening(id: number): void {
    if (id !== this.transitionId || !this.wantsVisible) return
    this.clearTransitionTimer()
    windowStateManager.setOverlayState(OverlayState.OPENED)
  }

  hideWindowBackend(force = false): void {
    if (!this.wantsVisible) return
    if (!this.rendererReady) {
      this.hideWindow()
      return
    }
    this.clearTransitionTimer()
    if (force) {
      this.wantsVisible = false
      windowStateManager.setOverlayState(OverlayState.CLOSING)
    }
    const id = force ? ++this.transitionId : this.transitionId
    WindowUtils.safelyEmitToWindow(this.overlayWindow, 'trigger-close-window', force, id)
    if (force) {
      // 动画完成确认丢失时仍能归还焦点，后续打开会取消这个兜底。
      this.transitionTimer = setTimeout(() => this.hideWindow(id), 250)
    }
  }

  hideWindow(id?: number): void {
    if (id !== undefined && id !== this.transitionId) return
    this.clearTransitionTimer()
    this.wantsVisible = false
    windowStateManager.setOverlayState(OverlayState.CLOSED)
    WindowUtils.safelyHideWindow(this.overlayWindow)
    WindowUtils.safelyEmitToWindow(this.overlayWindow, 'overlay-hidden', this.transitionId)
  }

  toggleWindow(force = false): void {
    if (this.wantsVisible) this.hideWindowBackend(force)
    else this.showWindow()
  }

  private clearTransitionTimer(): void {
    if (this.transitionTimer) clearTimeout(this.transitionTimer)
    this.transitionTimer = null
  }

  destroy(): void {
    this.clearTransitionTimer()
    WindowUtils.safelyDestroyWindow(this.overlayWindow)
    this.overlayWindow = null
  }

  isValid(): boolean {
    return WindowUtils.isWindowValid(this.overlayWindow)
  }

  isVisible(): boolean {
    return WindowUtils.isWindowValidAndVisible(this.overlayWindow)
  }

  updateWindowPositionBySettings(): void {
    if (!this.isValid()) return
    this.currentOverlayPosition = this.getOverlayPositionSetting()
    this.overlayWindow!.setBounds(
      WindowConfigCalculator.calculateWindowPosition(this.currentOverlayPosition)
    )
    WindowUtils.safelyEmitToWindow(
      this.overlayWindow,
      'overlay-position-updated',
      this.currentOverlayPosition
    )
  }
}

export const mainWindowManager = new MainWindowManager()
export const overlayWindowManager = new OverlayWindowManager()
