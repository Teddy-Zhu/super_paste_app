import { BrowserWindow } from 'electron'

/**
 * 前端通知服务
 * 统一管理所有前端通知逻辑，避免重复代码
 */
export class NotificationService {
  /**
   * 通知前端剪贴板历史已更新
   */
  static notifyClipboardHistoryUpdate(data: {
    timestamp: number
    source?: string
  }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', data)
        }
      })

      console.log('[通知服务] 已通知前端剪贴板历史更新:', data)
    } catch (error) {
      console.error('[通知服务] 通知前端剪贴板历史更新失败:', error)
    }
  }

  /**
   * 通知前端剪贴板变化
   */
  static notifyClipboardChange(data: {
    timestamp: number
  }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-changed', data)
        }
      })

      console.log('[通知服务] 已通知前端剪贴板变化:', data)
    } catch (error) {
      console.error('[通知服务] 通知前端剪贴板变化失败:', error)
    }
  }

  /**
   * 通知前端标签更新
   */
  static notifyTagsUpdate(data: {
    timestamp: number
    action?: string
  }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('tags-updated', data)
        }
      })

      console.log('[通知服务] 已通知前端标签更新:', data)
    } catch (error) {
      console.error('[通知服务] 通知前端标签更新失败:', error)
    }
  }

  /**
   * 通用的前端事件通知方法
   */
  static notifyFrontend(eventName: string, data: any): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send(eventName, data)
        }
      })

      console.log(`[通知服务] 已发送前端事件 ${eventName}:`, data)
    } catch (error) {
      console.error(`[通知服务] 发送前端事件 ${eventName} 失败:`, error)
    }
  }
}
