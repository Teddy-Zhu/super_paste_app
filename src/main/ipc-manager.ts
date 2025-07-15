import { ipcMain, BrowserWindow } from 'electron'
import clipboardManager from './clipboard-manager'
import type { ClipboardItem, ClipboardManagerConfig } from './types/clipboard-types'
import logger from './utils/logger'

/**
 * IPC 管理器
 * 负责注册和管理所有 IPC 事件处理器
 */
class IpcManager {
  private static instance: IpcManager
  private isInitialized: boolean = false

  private constructor() {}

  /**
   * 获取单例实例
   */
  public static getInstance(): IpcManager {
    if (!IpcManager.instance) {
      IpcManager.instance = new IpcManager()
    }
    return IpcManager.instance
  }

  /**
   * 通知前端标签数据已更新
   */
  private notifyTagsUpdated(): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('tags-updated', {
            timestamp: Date.now()
          })
        }
      })


    } catch (error) {
      logger.error('[IPC管理器] 通知前端标签更新失败:', error)
    }
  }

  /**
   * 通知前端剪贴板历史已更新
   */
  private notifyClipboardHistoryUpdated(): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', {
            timestamp: Date.now()
          })
        }
      })

    } catch (error) {
      logger.error('[IPC管理器] 通知前端剪贴板历史更新失败:', error)
    }
  }

  /**
   * 初始化所有 IPC 事件处理器
   */
  public initialize(): void {
    if (this.isInitialized) {

      return
    }

    this.registerClipboardHandlers()
    this.isInitialized = true


  }

  /**
   * 注册剪贴板相关的 IPC 事件处理器
   */
  private registerClipboardHandlers(): void {
    // 获取剪贴板历史记录
    ipcMain.handle('clipboard-get-history', async (_, options?: {
      limit?: number
      offset?: number
      tagIds?: string[]
      parseType?: string
      searchText?: string
      orderBy?: 'timestamp' | 'last_used'
      orderDirection?: 'ASC' | 'DESC'
    }) => {
      try {
        // 如果没有传入筛选选项，使用原有的简单获取方法
        if (!options || (Object.keys(options).length === 1 && 'limit' in options)) {
          const history = await clipboardManager.getHistory(options?.limit)
          return {
            success: true,
            data: history
          }
        }

        // 使用新的搜索方法
        const history = await clipboardManager.searchHistory(options)
        return {
          success: true,
          data: history
        }
      } catch (error) {
        logger.error('[IPC管理器] 获取剪贴板历史失败:', error)
        return {
          success: false,
          error: (error as Error).message || '获取剪贴板历史失败'
        }
      }
    })

    // 还原剪贴板内容
    ipcMain.handle('clipboard-restore', async (_, itemId: string) => {
      try {
        const success = await clipboardManager.restoreClipboard(itemId)
        return {
          success,
          message: success ? '剪贴板内容已还原' : '还原失败，未找到指定项目'
        }
      } catch (error) {
        logger.error('[IPC管理器] 还原剪贴板失败:', error)
        return {
          success: false,
          error: (error as Error).message || '还原剪贴板失败'
        }
      }
    })

    // 还原剪贴板内容为纯文本
    ipcMain.handle('clipboard-restore-as-text', async (_, itemId: string) => {
      try {
        const success = await clipboardManager.restoreClipboardAsText(itemId)
        return {
          success,
          message: success ? '剪贴板内容已还原为纯文本' : '还原失败，未找到指定项目或无文本内容'
        }
      } catch (error) {
        logger.error('[IPC管理器] 还原剪贴板为纯文本失败:', error)
        return {
          success: false,
          error: (error as Error).message || '还原剪贴板为纯文本失败'
        }
      }
    })

    // 删除剪贴板历史项
    ipcMain.handle('clipboard-delete-item', async (_, itemId: string) => {
      try {
        const success = await clipboardManager.deleteHistoryItem(itemId)
        return {
          success,
          message: success ? '剪贴板项已删除' : '删除失败，未找到指定项目'
        }
      } catch (error) {
        logger.error('[IPC管理器] 删除剪贴板项失败:', error)
        return {
          success: false,
          error: (error as Error).message || '删除剪贴板项失败'
        }
      }
    })

    // 清空剪贴板历史
    ipcMain.handle('clipboard-clear-history', async () => {
      try {
        await clipboardManager.clearHistory()
        return {
          success: true,
          message: '剪贴板历史已清空'
        }
      } catch (error) {
        logger.error('[IPC管理器] 清空剪贴板历史失败:', error)
        return {
          success: false,
          error: (error as Error).message || '清空剪贴板历史失败'
        }
      }
    })

    // 获取剪贴板管理器状态
    ipcMain.handle('clipboard-get-status', async () => {
      try {
        const isListening = await clipboardManager.isListeningActive()
        const config = await clipboardManager.getConfig()
        const history = await clipboardManager.getHistory()
        const historyCount = history.length
        
        return {
          success: true,
          data: {
            isListening,
            config,
            historyCount
          }
        }
      } catch (error) {
        logger.error('[IPC管理器] 获取剪贴板状态失败:', error)
        return {
          success: false,
          error: (error as Error).message || '获取剪贴板状态失败'
        }
      }
    })

    // 启动剪贴板监听
    ipcMain.handle('clipboard-start-listening', async () => {
      try {
        await clipboardManager.startListening()
        return {
          success: true,
          message: '剪贴板监听已启动'
        }
      } catch (error) {
        logger.error('[IPC管理器] 启动剪贴板监听失败:', error)
        return {
          success: false,
          error: (error as Error).message || '启动剪贴板监听失败'
        }
      }
    })

    // 停止剪贴板监听
    ipcMain.handle('clipboard-stop-listening', async () => {
      try {
        await clipboardManager.stopListening()
        return {
          success: true,
          message: '剪贴板监听已停止'
        }
      } catch (error) {
        logger.error('[IPC管理器] 停止剪贴板监听失败:', error)
        return {
          success: false,
          error: (error as Error).message || '停止剪贴板监听失败'
        }
      }
    })

    // 更新剪贴板管理器配置
    ipcMain.handle('clipboard-update-config', async (_, config: Partial<ClipboardManagerConfig>) => {
      try {
        await clipboardManager.updateConfig(config)
        return {
          success: true,
          message: '剪贴板配置已更新'
        }
      } catch (error) {
        logger.error('[IPC管理器] 更新剪贴板配置失败:', error)
        return {
          success: false,
          error: (error as Error).message || '更新剪贴板配置失败'
        }
      }
    })

    // 获取所有标签
    ipcMain.handle('get-all-tags', async () => {
      try {
        const tags = await clipboardManager.getAllTagGroups()
        return {
          success: true,
          data: tags
        }
      } catch (error) {
        logger.error('[IPC管理器] 获取所有标签失败:', error)
        return {
          success: false,
          error: (error as Error).message || '获取所有标签失败'
        }
      }
    })

    // 创建标签
    ipcMain.handle('create-tag', async (_, data: { name: string; color: string; description?: string }) => {
      try {
        const tagId = await clipboardManager.createTagGroup(data.name, data.color, data.description)
        if (tagId) {
          // 通知前端标签数据已更新
          this.notifyTagsUpdated()
          return {
            success: true,
            data: { tagId },
            message: '标签创建成功'
          }
        } else {
          return {
            success: false,
            error: '创建标签失败'
          }
        }
      } catch (error) {
        logger.error('[IPC管理器] 创建标签失败:', error)
        return {
          success: false,
          error: (error as Error).message || '创建标签失败'
        }
      }
    })

    // 更新标签
    ipcMain.handle('update-tag', async (_, data: { tagId: string; updates: { name?: string; color?: string; description?: string } }) => {
      try {
        const success = await clipboardManager.updateTagGroup(data.tagId, data.updates)
        if (success) {
          // 通知前端标签数据已更新
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签更新成功' : '更新标签失败'
        }
      } catch (error) {
        logger.error('[IPC管理器] 更新标签失败:', error)
        return {
          success: false,
          error: (error as Error).message || '更新标签失败'
        }
      }
    })

    // 删除标签
    ipcMain.handle('delete-tag', async (_, tagId: string) => {
      try {
        const success = await clipboardManager.deleteTagGroup(tagId)
        if (success) {
          // 通知前端标签数据已更新
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签删除成功' : '删除标签失败'
        }
      } catch (error) {
        logger.error('[IPC管理器] 删除标签失败:', error)
        return {
          success: false,
          error: (error as Error).message || '删除标签失败'
        }
      }
    })

    // 获取标签统计信息
    ipcMain.handle('get-tag-stats', async () => {
      try {
        const stats = await clipboardManager.getTagStats()
        return {
          success: true,
          data: stats
        }
      } catch (error) {
        logger.error('[IPC管理器] 获取标签统计信息失败:', error)
        return {
          success: false,
          error: (error as Error).message || '获取标签统计信息失败'
        }
      }
    })

    // 绑定标签到记录
    ipcMain.handle('bind-tag-to-record', async (_, data: { contentId: string; tagId: string }) => {
      try {
        const success = await clipboardManager.bindTagToRecord(data.contentId, data.tagId)
        if (success) {
          // 通知前端剪贴板历史已更新
          this.notifyClipboardHistoryUpdated()
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签绑定成功' : '绑定标签失败'
        }
      } catch (error) {
        logger.error('[IPC管理器] 绑定标签失败:', error)
        return {
          success: false,
          error: (error as Error).message || '绑定标签失败'
        }
      }
    })

    // 解除标签绑定
    ipcMain.handle('unbind-tag-from-record', async (_, data: { contentId: string; tagId: string }) => {
      try {
        const success = await clipboardManager.unbindTagFromRecord(data.contentId, data.tagId)
        if (success) {
          // 通知前端剪贴板历史已更新
          this.notifyClipboardHistoryUpdated()
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签解绑成功' : '解除标签绑定失败'
        }
      } catch (error) {
        logger.error('[IPC管理器] 解除标签绑定失败:', error)
        return {
          success: false,
          error: (error as Error).message || '解除标签绑定失败'
        }
      }
    })

  }

  /**
   * 移除所有 IPC 事件处理器
   */
  public cleanup(): void {
    if (!this.isInitialized) {
      return
    }

    // 移除剪贴板相关的处理器
    const clipboardChannels = [
      'clipboard-get-history',
      'clipboard-restore',
      'clipboard-restore-as-text',
      'clipboard-delete-item',
      'clipboard-clear-history',
      'clipboard-get-status',
      'clipboard-start-listening',
      'clipboard-stop-listening',
      'clipboard-update-config',
      'get-all-tags',
      'create-tag',
      'update-tag',
      'delete-tag',
      'get-tag-stats',
      'bind-tag-to-record',
      'unbind-tag-from-record'
    ]

    clipboardChannels.forEach(channel => {
      ipcMain.removeAllListeners(channel)
    })

    this.isInitialized = false
  }

  /**
   * 获取初始化状态
   */
  public getInitializationStatus(): boolean {
    return this.isInitialized
  }
}

// 导出单例实例
export default IpcManager.getInstance()
export { IpcManager }

// 导出类型定义供渲染进程使用
export interface ClipboardIpcResponse<T = any> {
  success: boolean
  data?: T
  message?: string
  error?: string
}

export interface ClipboardStatus {
  isListening: boolean
  config: ClipboardManagerConfig
  historyCount: number
}

export { type ClipboardItem }
