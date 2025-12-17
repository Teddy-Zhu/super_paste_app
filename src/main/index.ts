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
import { clearAllDebounceTimers } from './utils/common'
import { scheduledSyncManager } from './workers/scheduled-sync-manager'
import { BaseWorkerManager } from './workers/base-worker-manager'
import type { WorkerConfig } from './types/worker-types'
import clipboardHistoryCleanerWorkerPath from './workers/clipboard-history-cleaner.worker?modulePath'
import { getCleanupConfig } from './config/cleanup-config'

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

// 应用事件总线
import appEventBus from './utils/app-event-bus'

/**
 * 简单的清理器 Worker 管理器
 * 继承 BaseWorkerManager，不添加额外逻辑
 */
class ClipboardHistoryCleanerManager extends BaseWorkerManager {
  // 不需要额外实现，继承基类的所有功能
}

if (process.platform === 'darwin' && is.dev) {
  app.disableHardwareAcceleration()
}
app.commandLine.appendSwitch('disable-backgrounding-occluded-windows', 'true');
app.commandLine.appendSwitch('wm-window-animations-disabled', 'true');
/**
 * 应用程序主类
 */
class SuperPasteApp {
  private isInitialized = false // 标记应用是否已完成初始化
  private clipboardHistoryCleaner: ClipboardHistoryCleanerManager | null = null // 清理器 Worker 实例

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
   * 执行应用退出
   * 供托盘菜单等外部调用
   */
  public async performQuit(): Promise<void> {
    logger.info('[主进程] 开始执行应用退出流程')

    try {
      await this.cleanup()
      logger.info('[主进程] 清理完成,退出应用')
      app.exit(0)
    } catch (error) {
      logger.error('[主进程] 退出过程中发生错误:', error)
      // 即使出错也要退出
      app.exit(1)
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

    // 7. 注册应用退出事件监听器
    this.setupAppQuitHandler()
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
      const userDataPath = app.getPath('userData')
      clipboardDatabase.initialize(userDataPath)
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
      // 创建 Worker 配置
      const config: WorkerConfig = {
        name: 'ClipboardHistoryCleaner',
        workerPath: clipboardHistoryCleanerWorkerPath
      }

      // 创建 Worker 管理器实例
      this.clipboardHistoryCleaner = new ClipboardHistoryCleanerManager(config)

      // 启动 Worker
      await this.clipboardHistoryCleaner.start()
      logger.debug('[主进程] 剪贴板历史清理器 Worker 启动成功')

      // 获取用户数据路径
      const userDataPath = app.getPath('userData')

      // 获取清理配置
      const cleanupConfig = getCleanupConfig()

      // 初始化清理器配置
      await this.clipboardHistoryCleaner.call('initialize', {
        config: cleanupConfig,
        userDataPath
      })
      logger.debug('[主进程] 剪贴板历史清理器配置初始化完成')

      // 启动自动清理（如果配置了保留时间）
      if (cleanupConfig.enableAutoCleanup) {
        await this.clipboardHistoryCleaner.call('startAutoCleanup')
        logger.debug('[主进程] 剪贴板历史清理器自动清理启动完成')
      }
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
    // 退出清理标志 - 防止重复清理
    let cleanupCompleted = false

    // macOS 特有的激活事件
    app.on('activate', () => {
      // 只有在应用完全初始化后,且用户主动激活应用时才显示主窗口
      // 这样可以避免在应用启动时自动显示窗口
      if (!this.isInitialized) {
        return
      }

      this.ensureMainWindowVisible()
    })

    // 当所有窗口关闭时的处理
    app.on('window-all-closed', () => {
      logger.debug('[主进程] 所有窗口已关闭')
      // 在 macOS 上,除非明确退出,否则应用和菜单栏会保持活动状态
      // 但我们有系统托盘,所以不需要特殊处理 macOS
      // 如果用户点击了退出,app.quit() 会触发 will-quit 事件
    })

    // 在 app.quit() 被调用之前触发
    app.on('before-quit', () => {
      logger.info('[主进程] before-quit 事件触发')
    })

    // 应用即将退出时的清理
    app.on('will-quit', async (event) => {
      logger.info('[主进程] will-quit 事件触发')

      // 如果已经完成清理,直接退出
      if (cleanupCompleted) {
        logger.info('[主进程] 清理已完成,允许退出')
        return
      }

      // 阻止默认退出行为,等待清理完成
      event.preventDefault()

      logger.info('[主进程] ========== 应用即将退出,开始清理资源 ==========')

      try {
        await this.cleanup()
        logger.info('[主进程] ========== 资源清理完成,应用即将退出 ==========')
        cleanupCompleted = true
      } catch (error) {
        logger.error('[主进程] ========== 清理资源时发生错误 ==========', error)
        cleanupCompleted = true // 即使失败也标记为完成,避免卡住
      } finally {
        // 清理完成后,强制退出应用
        logger.info('[主进程] 调用 app.exit(0) 强制退出')
        app.exit(0)
      }
    })
  }

  /**
   * 设置应用退出事件处理器
   * 监听来自事件总线的退出请求
   */
  private setupAppQuitHandler(): void {
    appEventBus.onQuitRequested(async () => {
      logger.info('[主进程] 收到退出请求,开始执行退出流程')
      await this.performQuit()
    })
  }

  /**
   * 清理应用资源
   */
  public async cleanup(): Promise<void> {
    logger.info('[主进程] ========== 开始清理应用资源 ==========')

    try {
      // 1. 首先停止剪贴板监听(避免在清理过程中产生新的数据)
      logger.info('[主进程] [步骤 1/12] 开始停止剪贴板监听...')
      try {
        await Promise.race([
          clipboardManager.stopListening(),
          new Promise((_, reject) => setTimeout(() => reject(new Error('停止剪贴板监听超时')), 3000))
        ])
        logger.info('[主进程] [步骤 1/12] ✓ 剪贴板监听已停止')
      } catch (error) {
        logger.error('[主进程] [步骤 1/12] ✗ 停止剪贴板监听失败:', error)
      }

      // 2. 停止定时同步服务
      logger.info('[主进程] [步骤 2/12] 开始销毁定时同步服务...')
      try {
        await Promise.race([
          scheduledSyncManager.destroy(),
          new Promise((_, reject) => setTimeout(() => reject(new Error('销毁定时同步服务超时')), 3000))
        ])
        logger.info('[主进程] [步骤 2/12] ✓ 定时同步服务已销毁')
      } catch (error) {
        logger.error('[主进程] [步骤 2/12] ✗ 销毁定时同步服务失败:', error)
      }

      // 3. 销毁历史记录清理器
      logger.info('[主进程] [步骤 3/12] 开始销毁历史记录清理器...')
      if (this.clipboardHistoryCleaner) {
        try {
          await Promise.race([
            (async () => {
              logger.debug('[主进程] [步骤 3/12] 调用清理器 destroy 方法...')
              await this.clipboardHistoryCleaner!.call('destroy').catch((err) => {
                logger.warn('[主进程] [步骤 3/12] 清理器 destroy 调用失败:', err)
              })
              logger.debug('[主进程] [步骤 3/12] 停止清理器 Worker...')
              await this.clipboardHistoryCleaner!.stop()
              logger.debug('[主进程] [步骤 3/12] 清理器 Worker 已停止')
            })(),
            new Promise((_, reject) => setTimeout(() => reject(new Error('销毁历史清理器超时')), 3000))
          ])
          this.clipboardHistoryCleaner = null
          logger.info('[主进程] [步骤 3/12] ✓ 历史记录清理器已销毁')
        } catch (error) {
          logger.error('[主进程] [步骤 3/12] ✗ 销毁历史记录清理器失败:', error)
          // 强制停止
          if (this.clipboardHistoryCleaner) {
            logger.warn('[主进程] [步骤 3/12] 尝试强制停止清理器...')
            await this.clipboardHistoryCleaner.stop().catch(() => { })
            this.clipboardHistoryCleaner = null
          }
        }
      } else {
        logger.info('[主进程] [步骤 3/12] - 历史记录清理器不存在,跳过')
      }

      // 4. 停止剪贴板管理器 Worker
      logger.info('[主进程] [步骤 4/12] 开始停止剪贴板管理器 Worker...')
      try {
        await Promise.race([
          clipboardManager.stop(),
          new Promise((_, reject) => setTimeout(() => reject(new Error('停止剪贴板管理器超时')), 3000))
        ])
        logger.info('[主进程] [步骤 4/12] ✓ 剪贴板管理器已停止')
      } catch (error) {
        logger.error('[主进程] [步骤 4/12] ✗ 停止剪贴板管理器失败:', error)
      }

      // 5. 清理所有防抖定时器
      logger.info('[主进程] [步骤 5/12] 清理防抖定时器...')
      clearAllDebounceTimers()
      logger.info('[主进程] [步骤 5/12] ✓ 防抖定时器已清理')

      // 6. 关闭数据库连接
      logger.info('[主进程] [步骤 6/12] 关闭数据库连接...')
      try {
        clipboardDatabase.close()
        logger.info('[主进程] [步骤 6/12] ✓ 数据库连接已关闭')
      } catch (error) {
        logger.error('[主进程] [步骤 6/12] ✗ 关闭数据库连接失败:', error)
      }

      // 7. 清理 IPC 事件处理器
      logger.info('[主进程] [步骤 7/12] 清理 IPC 事件处理器...')
      try {
        ipcManager.cleanup()
        IPCHandlerManager.cleanup()
        logger.info('[主进程] [步骤 7/12] ✓ IPC 事件处理器已清理')
      } catch (error) {
        logger.error('[主进程] [步骤 7/12] ✗ 清理 IPC 事件处理器失败:', error)
      }

      // 8. 销毁托盘
      logger.info('[主进程] [步骤 8/12] 销毁系统托盘...')
      try {
        trayManager.destroy()
        logger.info('[主进程] [步骤 8/12] ✓ 系统托盘已销毁')
      } catch (error) {
        logger.error('[主进程] [步骤 8/12] ✗ 销毁系统托盘失败:', error)
      }

      // 9. 销毁窗口
      logger.info('[主进程] [步骤 9/12] 销毁窗口...')
      try {
        mainWindowManager.destroy()
        overlayWindowManager.destroy()
        logger.info('[主进程] [步骤 9/12] ✓ 窗口已销毁')
      } catch (error) {
        logger.error('[主进程] [步骤 9/12] ✗ 销毁窗口失败:', error)
      }

      // 10. 重置窗口状态
      logger.info('[主进程] [步骤 10/12] 重置窗口状态...')
      try {
        windowStateManager.reset()
        logger.info('[主进程] [步骤 10/12] ✓ 窗口状态已重置')
      } catch (error) {
        logger.error('[主进程] [步骤 10/12] ✗ 重置窗口状态失败:', error)
      }

      // 11. 清理电源监控
      logger.info('[主进程] [步骤 11/12] 清理电源监控...')
      try {
        powerMonitorManager.cleanup()
        logger.info('[主进程] [步骤 11/12] ✓ 电源监控已清理')
      } catch (error) {
        logger.error('[主进程] [步骤 11/12] ✗ 清理电源监控失败:', error)
      }

      // 12. 取消所有全局快捷键注册
      logger.info('[主进程] [步骤 12/12] 取消全局快捷键注册...')
      try {
        globalShortcutManager.unregisterAll()
        logger.info('[主进程] [步骤 12/12] ✓ 全局快捷键已取消注册')
      } catch (error) {
        logger.error('[主进程] [步骤 12/12] ✗ 取消全局快捷键注册失败:', error)
      }

      logger.info('[主进程] ========== 应用资源清理完成 ==========')
    } catch (error) {
      logger.error('[主进程] ========== 清理应用资源时发生严重错误 ==========', error)
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

// 导出应用实例,供其他模块使用(如托盘管理器)
export { superPasteApp }

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
