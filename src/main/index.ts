// Electron 核心模块
import { app } from 'electron'

// 首先初始化日志系统
import logger from './utils/logger'

// 第三方工具
import { electronApp, optimizer, is } from '@electron-toolkit/utils'

// 应用核心模块
import ipcManager from './communication/ipc-manager'
import clipboardManager from './clipboard/clipboard-manager'
import clipboardDatabase from './clipboard/clipboard-database'
import clipboardHistoryCleaner from './clipboard/clipboard-history-cleaner'
import { clearAllDebounceTimers } from './utils/common'
import { scheduledSyncManager } from './workers/scheduled-sync-manager'

// 重构后的模块管理器
import { mainWindowManager, overlayWindowManager } from './ui/windows/window-managers'
import { trayManager } from './ui/tray/tray-manager'
import { powerMonitorManager } from './system/power-monitor-manager'
import { globalShortcutManager } from './system/global-shortcut-manager'
import { windowStateManager } from './ui/windows/window-state'
import { SystemIntegrationService } from './system/initialization-services'
import { IPCHandlerManager } from './communication/ipc-handlers'
import { CLIPBOARD_CONFIG } from './config/constants'

// 同步相关模块
import { SyncConfigUtils } from './sync/sync-config-utils'
if (process.platform === 'darwin' && is.dev) {
  app.disableHardwareAcceleration()
}

/**
 * 应用程序主类
 */
class SuperPasteApp {
  private isInitialized = false // 标记应用是否已完成初始化

  /**
   * 应用启动入口
   */
  async start(): Promise<void> {
    try {
      // 初始化日志系统
      logger.initialize({
        userDataPath: app.getPath('userData'),
        fileName: 'main.log',
        scope: 'main'
      })

      logger.info('[主进程] Super Paste 应用开始启动')
      // 单实例锁定检查
      if (!this.acquireSingleInstanceLock()) {
        return
      }

      // 等待 Electron 准备就绪
      await app.whenReady()

      // 初始化应用
      await this.initialize()

      // 标记应用初始化完成
      this.isInitialized = true

      // 最后再次确保在 macOS 上 dock 是隐藏的
      if (process.platform === 'darwin') {
        app.dock?.hide()
      }

      logger.info('[主进程] Super Paste 应用启动完成')
    } catch (error) {
      logger.error('[主进程] 应用启动失败:', error)
      throw error
    }
  }

  /**
   * 获取单实例锁
   */
  private acquireSingleInstanceLock(): boolean {
    const gotTheLock = app.requestSingleInstanceLock()

    if (!gotTheLock) {
      logger.info('[主进程] 检测到应用已运行，退出当前实例')
      app.quit()
      return false
    }

    logger.info('[主进程] 获取单实例锁成功，继续启动应用')

    // 当第二个实例尝试启动时，聚焦到第一个实例
    app.on('second-instance', () => {
      logger.debug('[主进程] 检测到第二个实例启动尝试，聚焦到当前实例')
      this.handleSecondInstance()
    })

    return true
  }

  /**
   * 处理第二个实例启动
   */
  private handleSecondInstance(): void {
    this.ensureMainWindowVisible()
  }

  /**
   * 初始化应用
   */
  private async initialize(): Promise<void> {
    logger.debug('[主进程] Electron 准备就绪，开始应用初始化')

    // 设置应用用户模型 ID
    electronApp.setAppUserModelId('com.v2dawn.sp')

    // 在 macOS 上确保应用启动时 dock 是隐藏的
    if (process.platform === 'darwin') {
      app.dock?.hide()
    }

    // 1. 立即创建主窗口以提升启动速度（但不显示）
    this.createMainWindow()

    // 2. 创建系统托盘（同步，快速）
    this.createSystemTray()

    // 3. 异步初始化后台服务
    this.initializeBackgroundServices()

    // 4. 设置开发工具
    this.setupDevelopmentTools()

    // 5. 注册 IPC 事件处理器
    this.setupIPCHandlers()

    // 6. 设置应用事件监听器
    this.setupAppEventListeners()
  }

  /**
   * 创建主窗口
   */
  private createMainWindow(): void {
    logger.debug('[主进程] 优先创建主窗口以提升启动速度')
    mainWindowManager.createWindow()
  }

  /**
   * 确保主窗口存在并处于可见状态
   */
  private ensureMainWindowVisible(): void {
    if (!mainWindowManager.isValid()) {
      mainWindowManager.createWindow()
    }

    if (mainWindowManager.isMinimized()) {
      mainWindowManager.restore()
    }

    mainWindowManager.showWindow()
  }

  /**
   * 创建系统托盘
   */
  private createSystemTray(): void {
    logger.debug('[主进程] 创建系统托盘')
    trayManager.createTray()
  }

  /**
   * 异步初始化后台服务
   */
  private async initializeBackgroundServices(): Promise<void> {
    try {
      logger.debug('[主进程] 开始异步初始化后台服务')

      // 1. 初始化数据库（优先初始化，确保数据存储可用）
      await this.initializeDatabase()

      // 2. 初始化 IPC 管理器（快速，同步）
      this.initializeIPCManager()

      // 3. 初始化剪贴板管理器（可能耗时）
      await this.initializeClipboardManager()

      // 4. 初始化剪贴板历史清理器
      await this.initializeClipboardHistoryCleaner()

      // 5. 创建 overlay 窗口（预先创建，保持隐藏）
      this.createOverlayWindow()

      // 6. 并行执行系统集成与定时同步初始化
      const systemIntegrationTask = this.setupSystemIntegration().catch((error) => {
        logger.error('[主进程] 系统集成服务初始化失败:', error)
      })
      await Promise.all([systemIntegrationTask, this.initializeScheduledSync()])

      logger.debug('[主进程] 后台服务初始化全部完成')
    } catch (error) {
      logger.error('[主进程] 后台服务初始化失败:', error)
      // 即使部分初始化失败，也要确保应用能够正常运行
    }
  }

  /**
   * 初始化数据库
   */
  private async initializeDatabase(): Promise<void> {
    try {
      clipboardDatabase.initialize()
      logger.debug('[主进程] 数据库初始化完成')
    } catch (error) {
      logger.error('[主进程] 数据库初始化失败:', error)
      throw new Error('数据库初始化失败')
    }
  }

  /**
   * 初始化 IPC 管理器
   */
  private initializeIPCManager(): void {
    ipcManager.initialize()
    logger.debug('[主进程] IPC 管理器初始化完成')
  }

  /**
   * 初始化剪贴板管理器
   */
  private async initializeClipboardManager(): Promise<void> {
    await clipboardManager.initialize({
      checkInterval: CLIPBOARD_CONFIG.CHECK_INTERVAL,
      enableAutoSave: CLIPBOARD_CONFIG.ENABLE_AUTO_SAVE
    })
    logger.debug('[主进程] 剪贴板管理器初始化完成')

    await clipboardManager.startListening()
    logger.debug('[主进程] 剪贴板监听启动完成')
  }

  /**
   * 初始化剪贴板历史清理器
   */
  private async initializeClipboardHistoryCleaner(): Promise<void> {
    try {
      // 初始化清理器（启动worker并配置）
      await clipboardHistoryCleaner.initialize()
      logger.debug('[主进程] 剪贴板历史清理器Worker初始化完成')

      // 启动自动清理（如果配置了保留时间）
      await clipboardHistoryCleaner.startAutoCleanup()
      logger.debug('[主进程] 剪贴板历史清理器自动清理启动完成')
    } catch (error) {
      logger.error('[主进程] 剪贴板历史清理器初始化失败:', error)
      // 清理器初始化失败不影响应用主要功能
    }
  }

  /**
   * 设置系统集成功能
   */
  private async setupSystemIntegration(): Promise<void> {
    // 设置电源监控
    powerMonitorManager.setup()
    logger.debug('[主进程] 电源监控设置完成')

    // 初始化系统集成服务（标签数据、自启动等）
    await SystemIntegrationService.initialize()
    logger.debug('[主进程] 系统集成服务初始化完成')
  }

  /**
   * 创建 overlay 窗口
   */
  private createOverlayWindow(): void {
    overlayWindowManager.createWindow()
    logger.debug('[主进程] Overlay 窗口创建完成')
  }

  /**
   * 初始化定时同步服务
   */
  private async initializeScheduledSync(): Promise<void> {
    try {
      // 设置事件监听器
      scheduledSyncManager.setupEventListeners()

      // 总是加载基础配置，即使 provider 不可用
      const baseConfig = SyncConfigUtils.loadBaseSyncConfig()

      logger.info('[主进程] 开始初始化定时同步 Worker', baseConfig)

      // 初始化定时同步管理器（总是使用基础配置初始化 Worker）
      const initialized = await scheduledSyncManager.initialize(baseConfig)

      if (initialized) {
        if (baseConfig.enabled) {
          logger.info('[主进程] 定时同步 Worker 初始化成功，同步任务已启动')
        } else {
          logger.info('[主进程] 定时同步 Worker 基础功能初始化成功，等待完整配置')
        }
      } else {
        logger.warn('[主进程] 定时同步 Worker 初始化失败')
      }

      logger.debug('[主进程] 定时同步服务初始化完成')
    } catch (error) {
      logger.error('[主进程] 定时同步服务初始化失败:', error)
      // 同步服务初始化失败不影响应用主要功能
    }
  }

  /**
   * 设置开发工具
   */
  private setupDevelopmentTools(): void {
    // 在开发环境中启用 F12 开发者工具和忽略 Ctrl+R
    app.on('browser-window-created', (_, window) => {
      optimizer.watchWindowShortcuts(window)
    })
  }

  /**
   * 设置 IPC 事件处理器
   */
  private setupIPCHandlers(): void {
    IPCHandlerManager.initialize()
  }

  /**
   * 设置应用事件监听器
   */
  private setupAppEventListeners(): void {
    // macOS 特有的激活事件
    app.on('activate', () => {
      // 只有在应用完全初始化后，且用户主动激活应用时才显示主窗口
      // 这样可以避免在应用启动时自动显示窗口
      if (!this.isInitialized) {
        return
      }

      this.ensureMainWindowVisible()
    })

    // 当所有窗口关闭时不退出应用（因为有系统托盘）
    app.on('window-all-closed', () => {
      // 保持任务栏图标运行，用户可以通过托盘菜单退出
    })

    // 应用退出前清理资源
    app.on('before-quit', async () => {
      await this.cleanup()
    })
  }

  /**
   * 清理应用资源
   */
  private async cleanup(): Promise<void> {
    logger.debug('[主进程] 开始清理应用资源')

    try {
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
      IPCHandlerManager.cleanup()

      // 销毁托盘
      trayManager.destroy()

      // 销毁窗口
      mainWindowManager.destroy()
      overlayWindowManager.destroy()

      // 重置窗口状态
      windowStateManager.reset()

      // 清理电源监控
      powerMonitorManager.cleanup()

      // 取消所有全局快捷键注册
      globalShortcutManager.unregisterAll()

      // 销毁定时同步服务
      await scheduledSyncManager.destroy()

      logger.debug('[主进程] 应用资源清理完成')
    } catch (error) {
      logger.error('[主进程] 清理应用资源时发生错误:', error)
    }
  }
}

// ===== 应用启动 =====

// 开发环境配置
if (process.platform === 'darwin' && is.dev) {
  // app.disableHardwareAcceleration()
}

// 创建并启动应用
const superPasteApp = new SuperPasteApp()
superPasteApp.start().catch((error) => {
  logger.error('[主进程] 应用启动失败:', error)
  process.exit(1)
})

// ===== 模块化重构说明 =====
/*
本文件已完成模块化重构，主要改进包括：

1. 模块拆分：
   - constants.ts: 应用常量定义
   - window-state.ts: 窗口状态管理
   - global-shortcut-manager.ts: 全局快捷键管理
   - window-config.ts: 窗口配置工具
   - window-managers.ts: 窗口管理器
   - tray-manager.ts: 系统托盘管理
   - power-monitor-manager.ts: 电源监控管理
   - initialization-services.ts: 初始化服务
   - ipc-handlers.ts: IPC 事件处理

2. 面向对象设计：
   - 使用类封装相关功能
   - 提供单例实例便于使用
   - 清晰的职责分离

3. 改进的错误处理：
   - 统一的错误处理策略
   - 详细的日志记录
   - 优雅的降级处理

4. 更好的代码组织：
   - 清晰的初始化流程
   - 模块化的事件处理
   - 统一的资源清理

5. 增强的可维护性：
   - 松耦合的模块设计
   - 便于测试的结构
   - 清晰的接口定义
*/
