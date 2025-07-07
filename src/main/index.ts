import { app, shell, BrowserWindow, ipcMain, screen, globalShortcut, Tray, Menu } from 'electron'
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
import { STORE_KEY } from './common'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import { debounceExecution, clearAllDebounceTimers } from './utils/common'

let overlayWindow: BrowserWindow | null = null
let mainWindow: BrowserWindow | null = null
let tray: Tray | null = null

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
  overlayWindow.show()
  overlayWindow.setBounds(overlayWindow.getBounds());
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

function hideOverlayWindowBackend(): void {
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
  overlayWindow.webContents.send('trigger-close-window')
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
function toggleOverlayWindow(): void {
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
      hideOverlayWindowBackend()
      break

    case OverlayState.OPENING:
    case OverlayState.CLOSING:
      // 忽略切换请求
      break

    default:
      console.warn(`[主进程] 未知的overlay状态: ${currentState}`)
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
        toggleOverlayWindow()
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
    toggleOverlayWindow()
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
        toggleOverlayWindow()
      })

      if (!success) {
        console.error('[主进程] 全局快捷键注册失败:', shortcutKey)
      }
    }
  } catch (error) {
    console.error('[主进程] 初始化全局快捷键时发生错误:', error)
  }
}

function createWindow(): void {
  // Create the browser window.
  mainWindow = new BrowserWindow({
    width: 900,
    height: 670,
    show: false,
    hasShadow: false,
    autoHideMenuBar: true,
    ...(process.platform === 'linux' ? { icon: getWindowIcon() } : {}),
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false
    }
  })

  mainWindow.on('ready-to-show', () => {
    // 不自动显示主窗口，由用户通过任务栏图标手动打开
    // mainWindow?.show()
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
    mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL'])
  } else {
    mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
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
      contextIsolation: true
    }
  })
  overlayWindow.on('ready-to-show', () => {
    if (is.dev) {
      overlayWindow?.webContents.openDevTools()
    }

    // 在窗口准备就绪后初始化全局快捷键
    initializeGlobalShortcut()
    console.log('[主进程] 全局快捷键已初始化')
  })
  overlayWindow.on('show', () => {
    console.log('[主进程] 发送滑入动画触发信号')
    overlayWindow?.webContents.send('trigger-slide-in-animation')
  })
  overlayWindow.on('focus', () => {
    console.log('[✓] 窗口已在前台显示并获得焦点');
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
function createSampleTagsIfNeeded(): void {
  try {
    // 检查是否已有标签数据
    const existingTags = clipboardManager.getAllTagGroups()

    if (existingTags.length === 0) {
      console.log('[主进程] 创建示例标签数据')

      // 创建一些示例标签
      const sampleTags = [
        { name: 'Work', color: '#3b82f6', description: 'Work related content' },
        { name: 'Personal', color: '#10b981', description: 'Personal notes and content' },
        { name: 'Code', color: '#8b5cf6', description: 'Code snippets and programming' },
        { name: 'Links', color: '#f59e0b', description: 'Useful links and bookmarks' },
        { name: 'Ideas', color: '#ef4444', description: 'Creative ideas and inspiration' }
      ]

      sampleTags.forEach(tag => {
        const tagId = clipboardManager.createTagGroup(tag.name, tag.color, tag.description)
        if (tagId) {
          console.log(`[主进程] 创建标签成功: ${tag.name} (${tagId})`)
        } else {
          console.error(`[主进程] 创建标签失败: ${tag.name}`)
        }
      })
    } else {
      console.log(`[主进程] 已存在 ${existingTags.length} 个标签，跳过示例数据创建`)
    }
  } catch (error) {
    console.error('[主进程] 创建示例标签数据失败:', error)
  }
}

// 单实例锁定 - 确保只有一个应用实例运行
const gotTheLock = app.requestSingleInstanceLock()

if (!gotTheLock) {
  // 如果获取锁失败，说明已有实例在运行，退出当前实例
  console.log('[主进程] 应用已在运行，退出当前实例')
  app.quit()
} else {
  // 当第二个实例尝试启动时，聚焦到第一个实例
  app.on('second-instance', () => {
    console.log('[主进程] 检测到第二个实例启动，聚焦到当前实例')

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
  app.whenReady().then(() => {
    // Set app user model id for windows
    electronApp.setAppUserModelId('com.electron')

    // 创建任务栏图标
    createTray()

    // 初始化 IPC 管理器
    ipcManager.initialize()

    // 初始化剪贴板管理器
    clipboardManager.initialize({
      checkInterval: 200,
      enableAutoSave: true
    })

    // 启动剪贴板监听
    clipboardManager.startListening()

    // 创建示例标签数据（仅在开发环境或首次运行时）
    createSampleTagsIfNeeded()

    // 预先创建 overlay 窗口（初始化状态，保持隐藏）
    // 全局快捷键将在窗口的 ready-to-show 事件中初始化
    createOverlayWindow()

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

    // 监听前端动画完成消息
    ipcMain.on('open-overlay-finished', () => {
      if (getOverlayState() === OverlayState.OPENING) {
        setOverlayState(OverlayState.OPENED)
      }
    })

    // IPC handlers for store operations
    ipcMain.handle('store-get', (_, key: string, defaultValue?: any) => {
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

    ipcMain.handle('store-set', (_, key: string, value: any) => {
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
              console.log('[防抖] 执行最大历史记录数量配置更新')
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
              console.log('[防抖] 执行最大保存时间配置更新')
              clipboardManager.updateMaxKeepTimeFromStore()
            },
            2000 // 2000ms 防抖延迟
          )
        }

        return { success: true }
      } catch (error) {
        return { success: false, error: (error as Error).message }
      }
    })



    createWindow()

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
    console.log('[应用退出] 已清理所有防抖定时器')

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
