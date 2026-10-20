import { app, BrowserWindow, screen, shell, type NativeImage, type WebContents } from 'electron'
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
import { MAIN_WINDOW_CONFIG } from '../../config/constants'
import logger from '../../utils/logger'
import { isWayland } from '../../system/session'
import { OVERLAY_RENDERER_EVENTS, type OverlayRendererTrace } from '../../../shared/overlay-events'

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
        minWidth: MAIN_WINDOW_CONFIG.MIN_WIDTH,
        minHeight: MAIN_WINDOW_CONFIG.MIN_HEIGHT,
        frame: false,
        show: false, // 确保窗口创建时不显示
        center: true, // 窗口居中显示
        hasShadow: true,
        backgroundColor: '#f5f6f8',
        autoHideMenuBar: true,
        resizable: true,
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
  private transitionFrameContents: WebContents | null = null
  private appliedGeometry: { bounds: Electron.Rectangle; scaleFactor: number } | null = null
  private pendingOpenId: number | null = null
  private blurTimer: NodeJS.Timeout | null = null
  private pendingBlurId: number | null = null
  private transitionStartedAt = performance.now()
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
      backgroundColor: '#00000000',
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
    this.trace('window-created', {
      platform: process.platform,
      electron: process.versions.electron,
      session: process.env.XDG_SESSION_TYPE,
      desktop: process.env.XDG_CURRENT_DESKTOP,
      waylandSession: isWayland,
      ozonePlatform: app.commandLine.getSwitchValue('ozone-platform') || null,
      ozonePlatformHint:
        app.commandLine.getSwitchValue('ozone-platform-hint') ||
        process.env.ELECTRON_OZONE_PLATFORM_HINT ||
        null,
      development: is.dev
    })
    window.once('ready-to-show', () => {
      this.trace('native-ready')
      this.registerGlobalShortcut()
    })
    window.on('close', (event) => {
      event.preventDefault()
      this.hideWindowBackend(true, 'native-close')
    })
    window.on('closed', () => {
      this.trace('window-destroyed')
      this.clearTransitionTimer()
      this.clearBlurTimer()
      this.pendingOpenId = null
      this.rendererReady = false
      this.wantsVisible = false
      windowStateManager.reset()
      this.overlayWindow = null
      this.appliedGeometry = null
    })
    window.on('show', () =>
      this.trace('native-show', {
        bounds: window.getBounds(),
        contentBounds: window.getContentBounds(),
        alwaysOnTop: window.isAlwaysOnTop()
      })
    )
    window.on('hide', () => this.trace('native-hide'))
    window.on('focus', () => {
      this.clearBlurTimer()
      this.trace('native-focus')
    })
    window.on('blur', () => this.handleBlur())
    window.webContents.on('did-start-loading', () => {
      const pendingShow = this.wantsVisible
      this.rendererReady = false
      this.hideWindow(undefined, 'renderer-loading')
      this.wantsVisible = pendingShow
    })
    window.webContents.on('render-process-gone', (_event, details) => {
      this.trace('renderer-gone', { reason: details.reason, exitCode: details.exitCode })
      this.rendererReady = false
      this.hideWindow(undefined, 'renderer-gone')
    })
    const loading =
      is.dev && process.env['ELECTRON_RENDERER_URL']
        ? window.loadURL(`${process.env['ELECTRON_RENDERER_URL']}#/overlay`)
        : window.loadFile(join(__dirname, '../renderer/index.html'), { hash: '/overlay' })
    loading.catch((error) => logger.error('[Overlay] 页面加载失败:', error))
  }

  markRendererReady(): void {
    this.rendererReady = true
    this.trace('renderer-ready')
    if (this.wantsVisible) this.showWindow('renderer-ready')
  }

  private getOverlayPositionSetting(): OverlayPosition {
    return WindowConfigCalculator.normalizeOverlayPosition(
      store.get(STORE_KEY.SETTINGS_OVERLAY_POSITION, DEFAULT_OVERLAY_POSITION)
    )
  }

  private applyWindowBounds(bounds: Electron.Rectangle): boolean {
    if (!this.overlayWindow) return false
    const scaleFactor = screen.getDisplayMatching(bounds).scaleFactor
    const previous = this.appliedGeometry
    // Fractional scaling rounds native bounds differently. Reapplying the same
    // requested bounds repeatedly resizes the surface and discards its blank frame.
    if (
      previous?.scaleFactor === scaleFactor &&
      (['x', 'y', 'width', 'height'] as const).every((key) => previous.bounds[key] === bounds[key])
    ) {
      return false
    }
    this.overlayWindow.setBounds(bounds)
    this.appliedGeometry = { bounds: { ...bounds }, scaleFactor }
    return true
  }

  registerGlobalShortcut(retry = false): boolean {
    return globalShortcutManager.registerGlobalShortcut(
      store.get<string>(STORE_KEY.SETTINGS_GLOBAL_SHORTCUT, ''),
      () => this.toggleWindow(true, 'global-shortcut'),
      retry
    )
  }

  showWindow(source = 'api'): void {
    if (!this.isValid()) this.createWindow()
    this.wantsVisible = true
    if (!this.rendererReady || !this.overlayWindow) {
      this.trace('show-pending', { source, reason: 'renderer-not-ready' })
      return
    }
    const state = windowStateManager.getOverlayState()
    if (state === OverlayState.OPENING || state === OverlayState.OPENED) {
      this.trace('show-ignored', { source, reason: 'already-opening-or-opened' })
      return
    }

    this.clearTransitionTimer()
    this.clearBlurTimer()
    this.pendingOpenId = null
    const id = ++this.transitionId
    this.transitionStartedAt = performance.now()
    this.currentOverlayPosition = this.getOverlayPositionSetting()
    try {
      const bounds = WindowConfigCalculator.calculateWindowPosition(this.currentOverlayPosition)
      const previous = this.overlayWindow.getBounds()
      const boundsChanged = this.applyWindowBounds(bounds)
      windowStateManager.setOverlayState(OverlayState.OPENING)
      if (this.overlayWindow.isVisible()) {
        // 反向切换沿用当前画面，避免再次 show() 触发原生窗口动画。
        this.startOpening(id, false)
      } else {
        // 完全隐藏时先提交透明的动画首帧，防止 show() 暴露上一轮画面。
        this.pendingOpenId = id
        this.trace('prepare-request', {
          source,
          position: this.currentOverlayPosition,
          boundsChanged,
          requestedBounds: bounds,
          previousBounds: previous,
          actualBounds: this.overlayWindow.getBounds(),
          contentBounds: this.overlayWindow.getContentBounds(),
          displayScaleFactor: screen.getDisplayMatching(bounds).scaleFactor,
          zoomFactor: this.overlayWindow.webContents.getZoomFactor()
        })
        WindowUtils.safelyEmitToWindow(
          this.overlayWindow,
          'prepare-overlay-open',
          this.currentOverlayPosition,
          id
        )
        this.transitionTimer = setTimeout(() => {
          this.trace('prepare-timeout', { requestedId: id })
          this.hideWindow(id, 'prepare-timeout')
        }, 1000)
      }
    } catch (error) {
      logger.error('[Overlay] 显示失败:', error)
      this.hideWindow(id, 'show-error')
    }
  }

  showPreparedWindow(id: number): void {
    if (id !== this.transitionId || id !== this.pendingOpenId || !this.wantsVisible) {
      this.trace('prepare-ignored', { receivedId: id, reason: 'superseded' })
      return
    }
    try {
      this.waitForTransparentFrame(id, 'open')
    } catch (error) {
      logger.error('[Overlay] 显示准备好的窗口失败:', error)
      this.hideWindow(id, 'show-error')
    }
  }

  private startOpening(id: number, show: boolean): void {
    if (!this.overlayWindow || id !== this.transitionId || !this.wantsVisible) return
    this.clearTransitionTimer()
    this.pendingOpenId = null
    this.trace('open-start', { show })
    if (show) this.overlayWindow.show()
    else if (!this.overlayWindow.isFocused()) this.overlayWindow.focus()
    WindowUtils.safelyEmitToWindow(
      this.overlayWindow,
      'trigger-slide-in-animation',
      this.currentOverlayPosition,
      id
    )
    this.transitionTimer = setTimeout(() => this.finishOpening(id, 'animation-timeout'), 250)
  }

  finishOpening(id: number, source = 'renderer'): void {
    if (id !== this.transitionId || !this.wantsVisible || this.pendingOpenId !== null) {
      this.trace('open-ack-ignored', { receivedId: id, source, reason: 'superseded' })
      return
    }
    this.clearTransitionTimer()
    windowStateManager.setOverlayState(OverlayState.OPENED)
    this.trace('open-finished', { source })
    if (this.pendingBlurId === id) {
      this.pendingBlurId = null
      if (this.isVisible() && !this.overlayWindow?.isFocused()) {
        this.hideWindowBackend(true, 'blur-after-opening')
      }
    }
  }

  hideWindowBackend(force = false, source = 'api'): void {
    if (!this.wantsVisible) {
      this.trace('close-ignored', { source, reason: 'already-closing-or-closed' })
      return
    }
    if (!this.rendererReady) {
      this.hideWindow(undefined, 'renderer-not-ready')
      return
    }
    this.clearTransitionTimer()
    if (force) {
      this.clearBlurTimer()
      this.pendingOpenId = null
      this.wantsVisible = false
      windowStateManager.setOverlayState(OverlayState.CLOSING)
    }
    const id = force ? ++this.transitionId : this.transitionId
    if (force) this.transitionStartedAt = performance.now()
    if (force && !this.isVisible()) {
      this.hideWindow(id, 'closed-before-show')
      return
    }
    this.trace('close-command', { force, source })
    WindowUtils.safelyEmitToWindow(this.overlayWindow, 'trigger-close-window', force, id)
    if (force) {
      // 动画完成确认丢失时仍能归还焦点，后续打开会取消这个兜底。
      this.transitionTimer = setTimeout(() => this.hideWindow(id, 'animation-timeout'), 250)
    }
  }

  beginClosing(id: number): void {
    if (id !== this.transitionId) {
      this.trace('close-start-ignored', { receivedId: id, reason: 'superseded' })
      return
    }
    if (!this.wantsVisible) return
    this.clearTransitionTimer()
    this.clearBlurTimer()
    this.pendingOpenId = null
    this.wantsVisible = false
    this.transitionStartedAt = performance.now()
    windowStateManager.setOverlayState(OverlayState.CLOSING)
    this.trace('close-start', { source: 'renderer' })
    this.transitionTimer = setTimeout(() => this.hideWindow(id, 'animation-timeout'), 250)
  }

  finishClosing(id: number): void {
    if (id !== this.transitionId) {
      this.trace('close-ack-ignored', { receivedId: id, reason: 'superseded' })
      return
    }
    if (
      this.wantsVisible ||
      windowStateManager.getOverlayState() !== OverlayState.CLOSING ||
      !this.isVisible()
    ) {
      this.hideWindow(id)
      return
    }
    this.waitForTransparentFrame(id, 'close')
  }

  private waitForTransparentFrame(id: number, phase: 'open' | 'close'): void {
    if (!this.overlayWindow || this.transitionFrameContents) return

    this.clearTransitionTimer()
    const contents = this.overlayWindow.webContents
    const startedAt = performance.now()
    let frames = 0
    let invalidFrames = 0
    let nonTransparentFrames = 0
    let nonTransparentPixels: number | null = null
    let frameReady = false
    this.transitionTimer = setTimeout(
      () => {
        this.trace(`${phase}-frame-timeout`, {
          frames,
          invalidFrames,
          nonTransparentFrames,
          nonTransparentPixels
        })
        this.hideWindow(id, `${phase}-frame-timeout`)
      },
      phase === 'open' ? 250 : 80
    )
    const onFrame = (image: NativeImage): void => {
      if (
        frameReady ||
        id !== this.transitionId ||
        this.wantsVisible !== (phase === 'open') ||
        this.transitionFrameContents !== contents
      )
        return
      ++frames
      if (image.isEmpty()) {
        ++invalidFrames
        return
      }
      const bitmap = image.toBitmap()
      const { width, height } = image.getSize()
      if (!bitmap.length || bitmap.length !== width * height * 4) {
        ++invalidFrames
        return
      }
      nonTransparentPixels = 0
      // N32 位图的第 4 字节是 alpha；只检查透明度，不保存画面。
      for (let offset = 3; offset < bitmap.length; offset += 4) {
        if (bitmap[offset] !== 0) ++nonTransparentPixels
      }
      if (nonTransparentPixels !== 0) {
        ++nonTransparentFrames
        return
      }
      frameReady = true
      if (this.transitionTimer) clearTimeout(this.transitionTimer)
      this.transitionTimer = null
      this.trace(`${phase}-frame-ready`, {
        frames,
        invalidFrames,
        nonTransparentFrames,
        nonTransparentPixels,
        frameWidth: width,
        frameHeight: height,
        frameElapsedMs: Math.round(performance.now() - startedAt)
      })
      // 离开原生帧回调后再释放订阅；反向切换会使旧帧确认失效。
      setImmediate(() => {
        if (
          id === this.transitionId &&
          this.wantsVisible === (phase === 'open') &&
          this.transitionFrameContents === contents
        ) {
          if (phase === 'open') this.startOpening(id, true)
          else this.hideWindow(id, 'transparent-frame')
        }
      })
    }
    try {
      // Computed styles are not a GPU commit. Confirm an actual transparent
      // frame before either mapping the window or retaining it for the next open.
      this.transitionFrameContents = contents
      contents.beginFrameSubscription(false, onFrame)
      contents.invalidate()
      if (phase === 'open') {
        // A capturer keeps hidden page rendering active without showing its native window.
        void contents
          .capturePage(undefined, { stayHidden: false, stayAwake: true })
          .then(onFrame, (error: unknown) => {
            if (id === this.transitionId && this.transitionFrameContents === contents) {
              logger.warn('[Overlay] 隐藏窗口帧读取失败，继续等待绘制帧:', error)
            }
          })
      }
    } catch (error) {
      logger.error('[Overlay] 窗口帧确认失败:', error)
      this.hideWindow(id, `${phase}-frame-error`)
    }
  }

  hideWindow(id?: number, source = 'renderer'): void {
    if (id !== undefined && id !== this.transitionId) {
      this.trace('hide-ignored', { receivedId: id, source, reason: 'superseded' })
      return
    }
    this.clearTransitionTimer()
    this.clearBlurTimer()
    this.pendingOpenId = null
    this.wantsVisible = false
    windowStateManager.setOverlayState(OverlayState.CLOSED)
    this.trace('hide', { source })
    WindowUtils.safelyHideWindow(this.overlayWindow)
    WindowUtils.safelyEmitToWindow(this.overlayWindow, 'overlay-hidden', this.transitionId)
  }

  toggleWindow(force = false, source = 'api'): void {
    this.trace('toggle', { source, force, action: this.wantsVisible ? 'close' : 'open' })
    if (this.wantsVisible) this.hideWindowBackend(force, source)
    else this.showWindow(source)
  }

  private handleBlur(): void {
    this.trace('native-blur')
    if (is.dev) {
      this.trace('blur-ignored', { reason: 'development' })
      return
    }
    this.clearBlurTimer()
    const id = this.transitionId
    // 焦点事件可能晚于 hide/show；下一轮核对当前焦点和切换编号。
    this.blurTimer = setTimeout(() => {
      this.blurTimer = null
      if (id !== this.transitionId || !this.wantsVisible || !this.isVisible()) {
        this.trace('blur-ignored', { receivedId: id, reason: 'superseded-or-hidden' })
      } else if (this.overlayWindow?.isFocused()) {
        this.trace('blur-ignored', { reason: 'already-focused' })
      } else if (windowStateManager.getOverlayState() === OverlayState.OPENING) {
        this.pendingBlurId = id
        this.trace('blur-deferred', { reason: 'opening' })
      } else {
        this.hideWindowBackend(true, 'native-blur')
      }
    }, 0)
  }

  private clearBlurTimer(): void {
    if (this.blurTimer) clearTimeout(this.blurTimer)
    this.blurTimer = null
    this.pendingBlurId = null
  }

  private trace(event: string, details: Record<string, unknown> = {}): void {
    const window = this.isValid() ? this.overlayWindow : null
    logger.info(
      `[Overlay] ${JSON.stringify({
        event,
        id: this.transitionId,
        state: windowStateManager.getOverlayState(),
        wantsVisible: this.wantsVisible,
        visible: window?.isVisible() ?? false,
        focused: window?.isFocused() ?? false,
        rendererReady: this.rendererReady,
        pendingOpenId: this.pendingOpenId,
        elapsedMs: Math.round(performance.now() - this.transitionStartedAt),
        ...details
      })}`
    )
  }

  recordRendererTrace(data: unknown): void {
    if (!data || typeof data !== 'object') return
    const trace = data as Partial<OverlayRendererTrace>
    if (
      !trace.event ||
      !OVERLAY_RENDERER_EVENTS.includes(trace.event) ||
      !Number.isSafeInteger(trace.transitionId) ||
      typeof trace.sentAt !== 'number' ||
      !Number.isFinite(trace.sentAt)
    )
      return
    this.trace(`renderer-${trace.event}`, {
      rendererId: trace.transitionId,
      rendererVersion: Number.isSafeInteger(trace.animationVersion)
        ? trace.animationVersion
        : undefined,
      rendererState:
        typeof trace.animationState === 'string' ? trace.animationState.slice(0, 24) : undefined,
      rendererVisible: trace.overlayVisible === true,
      documentHidden: trace.documentHidden === true,
      documentFocused: trace.documentFocused === true,
      ipcDelayMs: Date.now() - trace.sentAt,
      rendererElapsedMs: Number.isFinite(trace.elapsedMs) ? trace.elapsedMs : undefined,
      receivedId: Number.isSafeInteger(trace.receivedId) ? trace.receivedId : undefined,
      reason: typeof trace.reason === 'string' ? trace.reason.slice(0, 160) : undefined,
      transform: typeof trace.transform === 'string' ? trace.transform.slice(0, 100) : undefined,
      viewportWidth: Number.isFinite(trace.viewportWidth) ? trace.viewportWidth : undefined,
      viewportHeight: Number.isFinite(trace.viewportHeight) ? trace.viewportHeight : undefined,
      devicePixelRatio: Number.isFinite(trace.devicePixelRatio) ? trace.devicePixelRatio : undefined
    })
  }

  private clearTransitionTimer(): void {
    if (this.transitionTimer) clearTimeout(this.transitionTimer)
    this.transitionTimer = null
    const contents = this.transitionFrameContents
    this.transitionFrameContents = null
    if (contents && !contents.isDestroyed()) contents.endFrameSubscription()
  }

  destroy(): void {
    this.clearTransitionTimer()
    this.clearBlurTimer()
    WindowUtils.safelyDestroyWindow(this.overlayWindow)
    this.overlayWindow = null
    this.appliedGeometry = null
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
    this.applyWindowBounds(
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
