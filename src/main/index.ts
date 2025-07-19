// 首先初始化日志系统
import logger from './utils/logger'

import { app, shell, BrowserWindow, ipcMain, screen, globalShortcut, Tray, Menu, powerMonitor } from 'electron'
import { join } from 'path'
import { electronApp, optimizer, is } from '@electron-toolkit/utils'
// 导入不同平台的图标
import iconPng from '../../resources/icon.png?asset'
import iconIco from '../../build/icon/icons/icon.ico?asset'
import iconIcns from '../../build/icon/icons/16x16.png?asset'
import iconLinux from '../../build/icon/icons/32x32.png?asset'
import store from './store'
import ipcManager from './ipc-manager'
import clipboardManager from './clipboard-manager'
import clipboardDatabase from './clipboard-database'
import databaseService from './services/database-service'
import { STORE_KEY } from './common'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import { debounceExecution, clearAllDebounceTimers } from './utils/common'
import autoStartManager from './utils/auto-start-manager'

let overlayWindow: BrowserWindow | null = null
let mainWindow: BrowserWindow | null = null
let tray: Tray | null = null

if (process.platform === 'darwin' && is.dev) {
  // app.disableHardwareAcceleration()
}
// Overlay窗口状态枚举
enum OverlayState {
  CLOSED = 'CLOSED',           // 窗口已关闭/隐藏
  OPENING = 'OPENING',         // 正在打开（显示窗口 + 滑入动画）
  OPENED = 'OPENED',           // 已打开（动画完成，可以交互）
  CLOSING = 'CLOSING'          // 正在关闭（滑出动画 + 隐藏窗口）
}

// 当前overlay状态
let currentOverlayState: OverlayState = OverlayState.CLOSED

/**
 * 设置overlay状态
 * @param newState 新状态
 */
function setOverlayState(newState: OverlayState): void {
  currentOverlayState = newState
}

/**
 * 获取当前overlay状态
 */
function getOverlayState(): OverlayState {
  return currentOverlayState
}

/**
 * 检查是否可以执行打开操作
 */
function canOpenOverlay(): boolean {
  return currentOverlayState === OverlayState.CLOSED
}

/**
 * 检查是否可以执行关闭操作
 */
function canCloseOverlay(): boolean {
  return currentOverlayState === OverlayState.OPENED
}

// 固定窗口高度
const WINDOW_HEIGHT = 350

/**
 * 获取当前显示器的窗口位置信息
 */
function calculateWindowPosition(): { x: number, y: number, width: number, height: number } {
  const cursorPoint = screen.getCursorScreenPoint()
  const currentDisplay = screen.getDisplayNearestPoint(cursorPoint)
  const { x: displayX, y: displayY, width, height } = currentDisplay.workArea

  // 窗口宽度为屏幕宽度，位置在屏幕最底部
  const windowWidth = width
  const windowX = displayX
  const windowY = displayY + height - WINDOW_HEIGHT

  return {
    x: windowX,
    y: windowY,
    width: windowWidth,
    height: WINDOW_HEIGHT
  }
}

/**
 * 根据屏幕大小计算最佳的主窗口尺寸
 */
function calculateMainWindowSize(): { width: number, height: number } {
  const primaryDisplay = screen.getPrimaryDisplay()
  const { width: screenWidth, height: screenHeight } = primaryDisplay.workArea

  // 根据屏幕尺寸计算最佳窗口大小
  // 宽度：屏幕宽度的 60-80%，最小 800px，最大 1200px
  let windowWidth = Math.floor(screenWidth * 0.7)
  windowWidth = Math.max(800, Math.min(1200, windowWidth))

  // 高度：屏幕高度的 70-85%，最小 600px，最大 900px
  let windowHeight = Math.floor(screenHeight * 0.75)
  windowHeight = Math.max(600, Math.min(900, windowHeight))



  return {
    width: windowWidth,
    height: windowHeight
  }
}

/**
 * 根据操作系统平台选择合适的托盘图标
 */
function getTrayIcon(): string {
  switch (process.platform) {
    case 'win32':
      return iconIco
    case 'darwin':
      return iconIcns
    case 'linux':
      return iconLinux
    default:
      return iconPng
  }
}

/**
 * 根据操作系统平台选择合适的窗口图标
 */
function getWindowIcon(): string {
  switch (process.platform) {
    case 'linux':
      return iconLinux
    default:
      return iconPng
  }
}

/**
 * 显示 overlay 窗口
 */
function showOverlayWindow(): void {
  if (!overlayWindow) {
    return
  }

  // 检查是否可以执行打开操作
  if (!canOpenOverlay()) {
    return
  }

  // 设置状态为正在打开
  setOverlayState(OverlayState.OPENING)

  // 显示并聚焦窗口
  overlayWindow.setBounds(overlayWindow.getBounds());
  overlayWindow.show()

  overlayWindow.focus()

  // 注册全局 ESC 键监听（如果还没注册的话）
  if (!globalShortcut.isRegistered('Escape')) {
    globalShortcut.register('Escape', () => {
      if (overlayWindow && overlayWindow.isVisible() && canCloseOverlay()) {
        overlayWindow.webContents.send('trigger-close-window')
      }
    })
  }
}

function hideOverlayWindowBackend(force = false): void {
  if (!overlayWindow) {
    return
  }

  // 检查是否可以执行关闭操作
  if (!canCloseOverlay()) {
    return
  }

  // 设置状态为正在关闭
  setOverlayState(OverlayState.CLOSING)

  // 发送关闭信号到前端
  overlayWindow.webContents.send('trigger-close-window', force)
}
/**
 * 隐藏 overlay 窗口
 */
function hideOverlayWindow(): void {
  if (!overlayWindow) {
    return
  }

  if (!overlayWindow.isVisible()) {
    // 确保状态正确
    setOverlayState(OverlayState.CLOSED)
    return
  }

  // 隐藏窗口
  overlayWindow.hide()

  // 取消全局 ESC 键监听
  if (globalShortcut.isRegistered('Escape')) {
    globalShortcut.unregister('Escape')
  }

  // 设置状态为已关闭
  setOverlayState(OverlayState.CLOSED)
}

/**
 * 切换 overlay 窗口显示状态
 * 如果窗口已显示则隐藏，否则显示
 */
function toggleOverlayWindow(force = false): void {
  if (!overlayWindow) {
    return
  }

  const currentState = getOverlayState()

  // 根据当前状态决定操作
  switch (currentState) {
    case OverlayState.CLOSED:
      showOverlayWindow()
      break

    case OverlayState.OPENED:
      hideOverlayWindowBackend(force)
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
 * 显示主窗口
 */
function showMainWindow(): void {
  if (!mainWindow) {
    createWindow()
    return
  }

  if (mainWindow.isDestroyed()) {
    createWindow()
    return
  }

  // 如果窗口存在但被隐藏，则显示它
  if (!mainWindow.isVisible()) {
    mainWindow.show()
  }

  // 确保窗口获得焦点
  mainWindow.focus()
}

/**
 * 创建任务栏图标
 */
function createTray(): void {
  if (tray) {
    return
  }

  // 创建任务栏图标
  tray = new Tray(getTrayIcon())

  // 设置任务栏图标的工具提示
  tray.setToolTip('Super Paste - 剪贴板管理工具')

  // 创建右键菜单
  const contextMenu = Menu.buildFromTemplate([
    {
      label: '设置',
      click: () => {
        showMainWindow()
      }
    },
    {
      label: '快捷面板',
      click: () => {
        toggleOverlayWindow(true)
      }
    },
    {
      type: 'separator'
    },
    {
      label: '退出',
      click: () => {
        app.quit()
      }
    }
  ])

  // 设置右键菜单
  tray.setContextMenu(contextMenu)

  // 设置左键点击事件（显示快捷面板）
  tray.on('click', () => {
    toggleOverlayWindow(true)
  })

  // 设置双击事件（显示主窗口）
  tray.on('double-click', () => {
    showMainWindow()
  })
}

/**
 * 初始化全局快捷键
 * 从store读取配置的全局快捷键，如果存在则注册监听
 */
function initializeGlobalShortcut(): void {
  try {
    // 从store读取全局快捷键配置
    const shortcutKey = store.get(STORE_KEY.SETTINGS_GLOBAL_SHORTCUT, '') as string

    if (shortcutKey && shortcutKey.trim() !== '') {
      // 注册全局快捷键监听 - 改为切换窗口显示状态
      const success = globalShortcut.register(shortcutKey, () => {
        toggleOverlayWindow(true)
      })

      if (success) {
        logger.debug(`[主进程] 全局快捷键注册成功: ${shortcutKey}`)
      } else {
        logger.error(`[主进程] 全局快捷键注册失败: ${shortcutKey}`)
      }
    } else {
      logger.debug('[主进程] 未配置全局快捷键，跳过注册')
    }
  } catch (error) {
    logger.error('[主进程] 初始化全局快捷键时发生错误:', error)
  }
}

function createWindow(): void {
  logger.debug('[主进程] 开始创建主窗口')

  // 计算最佳窗口尺寸
  const { width, height } = calculateMainWindowSize()

  // Create the browser window.
  mainWindow = new BrowserWindow({
    width,
    height,
    minWidth: width,
    minHeight: height,
    maxWidth: width,
    maxHeight: height,
    show: false,
    center: true, // 窗口居中显示
    hasShadow: false,
    autoHideMenuBar: true,
    resizable: false, // 禁止调整大小
    ...(process.platform === 'linux' ? { icon: getWindowIcon() } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false,
      nodeIntegration: false,
      contextIsolation: true
    }
  })

  mainWindow.on('ready-to-show', () => {
    logger.debug('[主进程] 主窗口准备就绪')
    // 不自动显示主窗口，由用户通过任务栏图标手动打开
    // mainWindow?.show()
    // if (is.dev) {
    //   mainWindow?.webContents.openDevTools()
    // }
  })

  // 监听主窗口显示事件
  mainWindow.on('show', () => {
    // 仅在 macOS 上控制 dock 显示
    if (process.platform === 'darwin') {
      app.dock?.show()
    }
  })

  // 监听主窗口隐藏事件
  mainWindow.on('hide', () => {
    // 仅在 macOS 上控制 dock 隐藏
    if (process.platform === 'darwin') {
      app.dock?.hide()
    }
  })

  // 监听主窗口关闭事件
  mainWindow.on('closed', () => {
    mainWindow = null
    // 仅在 macOS 上控制 dock 隐藏
    if (process.platform === 'darwin') {
      app.dock?.hide()
    }
  })

  mainWindow.webContents.setWindowOpenHandler((details) => {
    shell.openExternal(details.url)
    return { action: 'deny' }
  })
  // HMR for renderer base on electron-vite cli.
  // Load the remote URL for development or the local html file for production.
  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL']).then(() => {
      logger.debug('[主进程] 主窗口加载完成 (开发模式)')
    }).catch(error => {
      logger.error('[主进程] 主窗口加载失败 (开发模式):', error)
    })
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html')).then(() => {
      logger.debug('[主进程] 主窗口加载完成 (生产模式)')
    }).catch(error => {
      logger.error('[主进程] 主窗口加载失败 (生产模式):', error)
    })
  }
}

function createOverlayWindow(): void {
  if (overlayWindow) {
    return
  }

  // 计算窗口位置（居中显示）
  const position = calculateWindowPosition()

  // 创建覆盖层窗口
  overlayWindow = new BrowserWindow({
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
    focusable: true,
    show: false,
    type: 'utility',
    transparent: true,
    ...(process.platform === 'linux' ? { icon: getWindowIcon() } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false,
      nodeIntegration: false,
      contextIsolation: true,
      backgroundThrottling: false
    }
  })
  overlayWindow.on('ready-to-show', () => {
    if (is.dev) {
      overlayWindow?.webContents.openDevTools()
    }

    // 在窗口准备就绪后初始化全局快捷键（最后执行）
    logger.debug('[主进程] Overlay 窗口准备就绪，开始初始化全局快捷键')
    initializeGlobalShortcut()
    logger.debug('[主进程] 全局快捷键初始化完成，应用启动流程结束')
  })
  overlayWindow.on('show', () => {
    overlayWindow?.webContents.send('trigger-slide-in-animation')
  })
  overlayWindow.on('focus', () => {
    // 窗口已在前台显示并获得焦点
  });
  // 阻止窗口关闭，改为隐藏
  overlayWindow.on('close', (event) => {
    event.preventDefault()
    hideOverlayWindow()
  })

  overlayWindow.on('closed', () => {
    // 取消全局 ESC 键监听
    if (globalShortcut.isRegistered('Escape')) {
      globalShortcut.unregister('Escape')
    }

    // 重置overlay状态
    setOverlayState(OverlayState.CLOSED)

    overlayWindow = null
  })

  // 失去焦点时隐藏窗口
  overlayWindow.on('blur', () => {
    if (overlayWindow && !is.dev) {
      hideOverlayWindowBackend()
    }
  })
  overlayWindow.setSkipTaskbar(true)

  // 加载覆盖层页面
  if (is.dev && process.env['ELECTRON_RENDERER_URL']) {
    const url = `${process.env['ELECTRON_RENDERER_URL']}#/overlay`
    overlayWindow.loadURL(url)
  } else {
    overlayWindow.loadFile(join(__dirname, '../renderer/index.html'), { hash: '/overlay' })
  }


}

function closeOverlayWindow(): void {
  hideOverlayWindow()
}

/**
 * 创建示例标签数据（如果不存在）
 */
async function createSampleTagsIfNeeded(): Promise<void> {
  try {
    // 检查是否已有标签数据
    const existingTags = databaseService.getAllTagGroups()

    if (existingTags.length === 0) {
      // 创建一些示例标签
      const sampleTags = [
        { name: 'Work', color: '#3b82f6', description: 'Work related content' },
        { name: 'Personal', color: '#10b981', description: 'Personal notes and content' },
        { name: 'Code', color: '#8b5cf6', description: 'Code snippets and programming' },
        { name: 'Links', color: '#f59e0b', description: 'Useful links and bookmarks' },
        { name: 'Ideas', color: '#ef4444', description: 'Creative ideas and inspiration' }
      ]

      for (const tag of sampleTags) {
        try {
          const tagId = databaseService.createTagGroup({ name: tag.name, color: tag.color, description: tag.description })
          if (!tagId) {
            logger.error(`[主进程] 创建标签失败: ${tag.name}`)
          }
        } catch (error) {
          logger.error(`[主进程] 创建标签失败: ${tag.name}`, error)
        }
      }
    }
  } catch (error) {
    logger.error('[主进程] 创建示例标签数据失败:', error)
  }
}

/**
 * 同步自启状态
 * 检查配置中的自启设置与系统实际状态是否一致，如果不一致则同步
 */
async function syncAutoStartStatus(): Promise<void> {
  try {
    // 获取配置中的自启状态
    const configuredAutoStart = store.get(STORE_KEY.SETTINGS_AUTO_START, false) as boolean

    // 获取系统中的实际自启状态
    const actualAutoStart = await autoStartManager.isEnabled()

    logger.debug(`[主进程] 自启状态检查 - 配置: ${configuredAutoStart}, 实际: ${actualAutoStart}`)

    // 如果状态不一致，则根据配置更新系统状态
    if (configuredAutoStart !== actualAutoStart) {
      logger.debug(`[主进程] 自启状态不一致，正在同步到配置状态: ${configuredAutoStart}`)

      const success = await autoStartManager.setEnabled(configuredAutoStart)

      if (success) {
        logger.debug(`[主进程] 自启状态同步成功`)
      } else {
        logger.error(`[主进程] 自启状态同步失败`)
      }
    } else {
      logger.debug(`[主进程] 自启状态已同步`)
    }
  } catch (error) {
    logger.error('[主进程] 同步自启状态失败:', error)
  }
}

/**
 * 设置系统电源监控
 * 处理系统休眠和唤醒事件，确保剪贴板管理器在系统唤醒后正常工作
 */
function setupPowerMonitor(): void {
  try {
    // 监听系统挂起事件
    powerMonitor.on('suspend', () => {
      logger.debug('[主进程] 系统进入休眠状态')

      // 暂停剪贴板监听以避免在休眠期间的错误
      clipboardManager.stopListening().catch(error => {
        logger.error('[主进程] 停止剪贴板监听失败:', error)
      })
    })

    // 监听系统恢复事件
    powerMonitor.on('resume', () => {
      logger.debug('[主进程] 系统从休眠中恢复')

      // 延迟处理系统恢复，给系统一些时间完全恢复
      setTimeout(async () => {
        try {
          await clipboardManager.handleSystemResume()
          logger.debug('[主进程] 系统唤醒恢复处理完成')
        } catch (error) {
          logger.error('[主进程] 系统唤醒恢复处理失败:', error)
        }
      }, 2000) // 延迟2秒
    })

    // 监听系统锁定事件（可选）
    powerMonitor.on('lock-screen', () => {
      logger.debug('[主进程] 系统屏幕已锁定')
    })

    // 监听系统解锁事件（可选）
    powerMonitor.on('unlock-screen', () => {
      logger.debug('[主进程] 系统屏幕已解锁')
    })

    logger.debug('[主进程] 电源监控已设置')
  } catch (error) {
    logger.error('[主进程] 设置电源监控失败:', error)
  }
}

/**
 * 异步初始化后台服务
 * 将耗时的初始化操作放在后台执行，不阻塞主窗口显示
 */
async function initializeBackgroundServices(): Promise<void> {
  try {
    logger.debug('[主进程] 开始异步初始化后台服务')

    // 1. 初始化数据库（优先初始化，确保数据存储可用）
    try {
      clipboardDatabase.initialize()
      logger.debug('[主进程] 数据库初始化完成')
    } catch (error) {
      logger.error('[主进程] 数据库初始化失败:', error)
      throw new Error('数据库初始化失败')
    }

    // 2. 初始化 IPC 管理器（快速，同步）
    ipcManager.initialize()
    logger.debug('[主进程] IPC 管理器初始化完成')

    // 3. 初始化剪贴板管理器（可能耗时）
    await clipboardManager.initialize({
      checkInterval: 200,
      enableAutoSave: true
    })
    logger.debug('[主进程] 剪贴板管理器初始化完成')

    // 4. 启动剪贴板监听
    await clipboardManager.startListening()
    logger.debug('[主进程] 剪贴板监听启动完成')

    // 5. 设置系统休眠和唤醒事件监听
    setupPowerMonitor()
    logger.debug('[主进程] 电源监控设置完成')

    // 6. 创建示例标签数据（仅在开发环境或首次运行时）
    await createSampleTagsIfNeeded()
    logger.debug('[主进程] 示例标签数据创建完成')

    // 7. 检查并同步自启状态
    await syncAutoStartStatus()
    logger.debug('[主进程] 自启状态同步完成')

    // 8. 预先创建 overlay 窗口（初始化状态，保持隐藏）
    createOverlayWindow()
    logger.debug('[主进程] Overlay 窗口创建完成')

    // 9. 最后初始化全局快捷键（确保所有依赖都已就绪）
    // 注意：全局快捷键会在 overlay 窗口的 ready-to-show 事件中初始化
    // 这里不需要额外调用，因为 createOverlayWindow() 已经处理了

    logger.debug('[主进程] 后台服务初始化全部完成')
  } catch (error) {
    logger.error('[主进程] 后台服务初始化失败:', error)
    // 即使部分初始化失败，也要确保应用能够正常运行
    // 可以考虑显示错误通知给用户
  }
}

// 单实例锁定 - 确保只有一个应用实例运行
const gotTheLock = app.requestSingleInstanceLock()

if (!gotTheLock) {
  // 如果获取锁失败，说明已有实例在运行，退出当前实例

  app.quit()
} else {
  // 当第二个实例尝试启动时，聚焦到第一个实例
  app.on('second-instance', () => {

    // 如果有主窗口，显示并聚焦主窗口
    if (mainWindow) {
      if (mainWindow.isMinimized()) {
        mainWindow.restore()
      }
      if (!mainWindow.isVisible()) {
        mainWindow.show()
      }
      mainWindow.focus()
    } else {
      // 如果主窗口不存在，创建并显示
      createWindow()
    }

    // 也可以选择显示 overlay 窗口
    // toggleOverlayWindow()
  })

  // This method will be called when Electron has finished
  // initialization and is ready to create browser windows.
  // Some APIs can only be used after this event occurs.
  app.whenReady().then(async () => {
    logger.debug('[主进程] Electron 准备就绪，开始应用初始化')

    // Set app user model id for windows
    electronApp.setAppUserModelId('com.v2dawn.sp')

    // 立即创建主窗口以提升启动速度
    logger.debug('[主进程] 优先创建主窗口以提升启动速度')
    createWindow()

    // 创建任务栏图标（同步，快速）
    logger.debug('[主进程] 创建任务栏图标')
    createTray()

    // 异步初始化其他组件，不阻塞主窗口显示
    logger.debug('[主进程] 开始异步初始化后台服务')
    initializeBackgroundServices()

    // Default open or close DevTools by F12 in development
    // and ignore CommandOrControl + R in production.
    // see https://github.com/alex8088/electron-toolkit/tree/master/packages/utils
    app.on('browser-window-created', (_, window) => {
      optimizer.watchWindowShortcuts(window)
    })

    // IPC handlers for overlay window
    ipcMain.on('open-overlay-window', () => {
      if (overlayWindow) {
        // 窗口已存在，切换显示状态
        toggleOverlayWindow()
      } else {
        // 窗口不存在，创建新窗口并显示
        createOverlayWindow()
        showOverlayWindow()
      }
    })

    ipcMain.on('close-overlay-window', () => {
      closeOverlayWindow()
    })

    // IPC handler for auto-start management
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
        return { success: false, error: (error as Error).message }
      }
    })

    ipcMain.handle('auto-start-set-status', async (_, enabled: boolean) => {
      try {
        const success = await autoStartManager.setEnabled(enabled)
        return { success, data: { enabled } }
      } catch (error) {
        return { success: false, error: (error as Error).message }
      }
    })

    ipcMain.handle('auto-start-toggle', async () => {
      try {
        const newState = await autoStartManager.toggle()
        return { success: true, data: { enabled: newState } }
      } catch (error) {
        return { success: false, error: (error as Error).message }
      }
    })

    // IPC handler for opening main window (settings)
    ipcMain.on('open-main-window', () => {
      showMainWindow()
    })

    // 监听前端动画完成消息
    ipcMain.on('open-overlay-finished', () => {
      if (getOverlayState() === OverlayState.OPENING) {
        setOverlayState(OverlayState.OPENED)
      }
    })

    // IPC handlers for store operations
    ipcMain.handle('store-get', (_, key: string, defaultValue?: unknown) => {
      try {
        return { success: true, data: store.get(key, defaultValue) }
      } catch (error) {
        return { success: false, error: (error as Error).message }
      }
    })
    ipcMain.on('store-get-on', (event, key) => {
      const val = store?.get(key)
      event.returnValue = val
    })

    ipcMain.handle('store-set', async (_, key: string, value: unknown) => {
      try {
        store.raw_set(key, value)

        // 如果更新的是全局快捷键配置，重新初始化快捷键
        if (key === STORE_KEY.SETTINGS_GLOBAL_SHORTCUT) {
          // 先取消所有已注册的快捷键（除了ESC键，它是在overlay窗口中注册的）
          globalShortcut.unregisterAll()
          // 重新初始化全局快捷键
          initializeGlobalShortcut()
        }

        // 如果更新的是最大历史记录数量配置，使用防抖更新剪贴板管理器配置
        if (key === STORE_KEY.SETTINGS_MAX_HISTORY_SIZE) {
          debounceExecution(
            'update-max-history-size',
            () => {
              clipboardManager.updateMaxHistorySizeFromStore()
            },
            800 // 800ms 防抖延迟
          )
        }

        // 如果更新的是最大保存时间配置，使用防抖更新历史记录清理器配置
        if (key === STORE_KEY.SETTINGS_MAX_KEEP_TIME) {
          debounceExecution(
            'update-max-keep-time',
            () => {
              clipboardManager.updateMaxKeepTimeFromStore()
            },
            2000 // 2000ms 防抖延迟
          )
        }

        // 如果更新的是自动启动配置，设置应用的自动启动状态
        if (key === STORE_KEY.SETTINGS_AUTO_START) {
          const autoStart = value as boolean
          const success = await autoStartManager.setEnabled(autoStart)
          if (!success) {
            logger.error('[主进程] 设置自动启动失败')
          }
        }

        return { success: true }
      } catch (error) {
        return { success: false, error: (error as Error).message }
      }
    })

    app.on('activate', function () {
      // On macOS it's common to re-create a window in the app when the
      // dock icon is clicked and there are no other windows open.
      if (BrowserWindow.getAllWindows().length === 0) {
        createWindow()
      } else if (mainWindow && !mainWindow.isDestroyed()) {
        // 如果主窗口存在但被隐藏，则显示它
        if (!mainWindow.isVisible()) {
          mainWindow.show()
        }
        // 确保窗口获得焦点
        mainWindow.focus()
      }
    })
  })

  // 当所有窗口关闭时不退出应用，因为我们有任务栏图标
  // 用户可以通过任务栏图标的退出菜单来退出应用
  app.on('window-all-closed', () => {
    // 不自动退出应用，保持任务栏图标运行
    // if (process.platform !== 'darwin') {
    //   app.quit()
    // }
  })

  // 应用退出前清理资源
  app.on('before-quit', () => {
    // 停止剪贴板监听
    clipboardManager.stopListening()

    // 销毁历史记录清理器
    clipboardHistoryCleaner.destroy()

    // 清理所有防抖定时器
    clearAllDebounceTimers()

    // 关闭数据库连接
    clipboardDatabase.close()

    // 清理 IPC 事件处理器
    ipcManager.cleanup()

    // 销毁任务栏图标
    if (tray && !tray.isDestroyed()) {
      tray.destroy()
      tray = null
    }

    // 销毁主窗口
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.destroy()
      mainWindow = null
    }

    // 销毁 overlay 窗口
    if (overlayWindow && !overlayWindow.isDestroyed()) {
      overlayWindow.destroy()
      overlayWindow = null
    }

    // 重置overlay状态
    setOverlayState(OverlayState.CLOSED)

    // 取消所有全局快捷键注册
    globalShortcut.unregisterAll()
  })

} // 结束单实例锁定的 else 块

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and require them here.
