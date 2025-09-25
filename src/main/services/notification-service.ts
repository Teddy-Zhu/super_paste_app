import { BrowserWindow } from 'electron'
import logger from '../utils/logger'

/**
 * 前端通知服务
 * 统一管理所有前端通知逻辑，避免重复代码
 */
export class NotificationService {
  /**
   * 通知前端剪贴板历史已更新
   */
  static notifyClipboardHistoryUpdate(data: { timestamp: number; source?: string }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', data)
        }
      })
    } catch (error) {
      logger.error('[通知服务] 通知前端剪贴板历史更新失败:', error)
    }
  }

  /**
   * 通知前端剪贴板变化
   */
  static notifyClipboardChange(data: { timestamp: number }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-changed', data)
        }
      })
    } catch (error) {
      logger.error('[通知服务] 通知前端剪贴板变化失败:', error)
    }
  }

  /**
   * 通知前端标签更新
   */
  static notifyTagsUpdate(data: { timestamp: number; action?: string }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('tags-updated', data)
        }
      })
    } catch (error) {
      logger.error('[通知服务] 通知前端标签更新失败:', error)
    }
  }

  /**
   * 通知前端单条记录OCR更新
   */
  static notifyRecordOCRUpdate(data: {
    contentId: string
    ocrText: string
    timestamp: number
  }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('record-ocr-updated', data)
        }
      })

      logger.debug('[通知服务] 已通知前端记录OCR更新:', data.contentId)
    } catch (error) {
      logger.error('[通知服务] 通知前端记录OCR更新失败:', error)
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
    } catch (error) {
      logger.error(`[通知服务] 发送前端事件 ${eventName} 失败:`, error)
    }
  }

  /**
   * 通知前端同步完成，需要重启应用
   */
  static notifySyncRequiresRestart(data: { downloadedFiles: string[]; message: string }): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('sync-requires-restart', data)
        }
      })

      logger.info('[通知服务] 已通知前端同步完成，需要重启应用')
    } catch (error) {
      logger.error('[通知服务] 通知前端同步需要重启失败:', error)
    }
  }
}
