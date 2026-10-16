import ActiveWindow, { WindowInfo } from '@paymoapp/active-window'
import logger from '../utils/logger'
import { EventEmitter } from 'events'
import { isWayland } from '../system/platform'

export interface ActiveWindowInfo {
  os: string
  windowClass: string
  app: string // Application name
  title: string
  pid: number
  bounds: {
    x: number
    y: number
    width: number
    height: number
  }
}

class ActiveWindowService extends EventEmitter {
  private static instance: ActiveWindowService
  private isInitialized: boolean = false
  private subscriptionId: number | null = null
  private currentWindow: WindowInfo | null = null

  private constructor() {
    super()
  }

  public static getInstance(): ActiveWindowService {
    if (!ActiveWindowService.instance) {
      ActiveWindowService.instance = new ActiveWindowService()
    }
    return ActiveWindowService.instance
  }

  /**
   * 检测窗口信息是否发生变化
   * 比较关键字段: title, application, path, pid
   */
  private isWindowChanged(oldWindow: WindowInfo | null, newWindow: WindowInfo): boolean {
    if (!oldWindow) return true

    return (
      oldWindow.title !== newWindow.title ||
      oldWindow.application !== newWindow.application ||
      oldWindow.path !== newWindow.path ||
      oldWindow.pid !== newWindow.pid
    )
  }

  public initialize(): void {
    if (this.isInitialized) return
    // 该原生库仅支持 X11，Wayland 下不读取错误的来源信息，也不申请无效权限。
    if (isWayland) {
      logger.info('[ActiveWindowService] Wayland 不支持活动窗口来源识别')
      return
    }
    logger.debug('[ActiveWindowService] Initializing')
    try {
      ActiveWindow.initialize()

      if (process.platform === 'darwin') {
        const hasPermission = ActiveWindow.requestPermissions()
        if (!hasPermission) {
          logger.warn('[ActiveWindowService] macOS Screen Recording permission required')
        }
      }

      this.subscriptionId = ActiveWindow.subscribe((windowInfo: WindowInfo | null) => {
        // 忽略 null 窗口信息
        if (!windowInfo) {
          return
        }

        // 检测窗口信息是否真正发生变化
        if (!this.isWindowChanged(this.currentWindow, windowInfo)) {
          return
        }

        this.currentWindow = windowInfo
        this.emit('active-window-change', windowInfo)
        logger.debug('[ActiveWindowService] Window changed:', windowInfo.application)
      })

      this.isInitialized = true
      logger.info('[ActiveWindowService] Initialized')
    } catch (error) {
      logger.error('[ActiveWindowService] Initialization failed:', error)
    }
  }

  public getCurrentWindow(): WindowInfo | null {
    return this.currentWindow
  }

  public cleanup(): void {
    if (this.subscriptionId !== null) {
      try {
        ActiveWindow.unsubscribe(this.subscriptionId)
        this.subscriptionId = null
      } catch (error) {
        logger.error('[ActiveWindowService] Cleanup failed:', error)
      }
    }
    this.isInitialized = false
  }
}

export const activeWindowService = ActiveWindowService.getInstance()
