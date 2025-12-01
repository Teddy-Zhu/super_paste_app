import { ipcMain, BrowserWindow } from 'electron'
import type { IpcMainInvokeEvent } from 'electron'
import clipboardManager from '../clipboard/clipboard-manager'
import clipboardHistoryService from '../services/clipboard-history-service'
import clipboardTagService from '../services/clipboard-tag-service'
import { qrCodeService } from '../services/qrcode-service'
import type { ClipboardItem, ClipboardManagerConfig } from '../types/clipboard-types'
import type { BaseWorkerManager } from '../workers/base-worker-manager'
import logger from '../utils/logger'
import { writeHeapSnapshot } from 'v8'
import { join } from 'path'
import { app } from 'electron'

/**
 * IPC 管理器
 * 负责注册和管理所有 IPC 事件处理器
 */
class IpcManager {
  private static instance: IpcManager
  private isInitialized: boolean = false

  private constructor() {
    // 保持单例构造器不可被外部调用
  }

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
   * 统一广播工具
   */
  private broadcastToAllWindows(channel: string, payload: Record<string, unknown> = {}): void {
    try {
      const message = { timestamp: Date.now(), ...payload }
      BrowserWindow.getAllWindows().forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send(channel, message)
        }
      })
    } catch (error) {
      logger.error(`[IPC管理器] 广播事件失败 [${channel}]:`, error)
    }
  }

  /**
   * 通用 IPC handler 注册工具
   */
  private registerHandler<TArgs extends unknown[]>(
    channel: string,
    handler: (event: IpcMainInvokeEvent, ...args: TArgs) => Promise<unknown>,
    context: string
  ): void {
    ipcMain.handle(channel, async (event, ...args) => {
      try {
        return await handler(event, ...(args as TArgs))
      } catch (error) {
        logger.error(`[IPC管理器] ${context}失败:`, error)
        return {
          success: false,
          error: (error as Error).message || `${context}失败`
        }
      }
    })
  }

  /**
   * 通知前端标签数据已更新
   */
  private notifyTagsUpdated(): void {
    this.broadcastToAllWindows('tags-updated')
  }

  /**
   * 通知前端剪贴板历史已更新
   */
  private notifyClipboardHistoryUpdated(): void {
    this.broadcastToAllWindows('clipboard-history-updated')
  }

  /**
   * 按需通知前端状态变化
   */
  private notifyStateChange(options: { history?: boolean; tags?: boolean }): void {
    const { history, tags } = options
    if (history) {
      this.notifyClipboardHistoryUpdated()
    }
    if (tags) {
      this.notifyTagsUpdated()
    }
  }

  /**
   * 初始化所有 IPC 事件处理器
   */
  public initialize(): void {
    if (this.isInitialized) {
      logger.debug('[IPC管理器] 已初始化，跳过重复初始化')
      return
    }

    // 在初始化前先清理可能存在的旧处理器
    this.cleanup()

    this.registerClipboardHandlers()
    this.isInitialized = true

    logger.debug('[IPC管理器] 初始化完成')
  }

  /**
   * 注册剪贴板相关的 IPC 事件处理器
   */
  private registerClipboardHandlers(): void {
    this.registerHistoryHandlers()
    this.registerClipboardControlHandlers()
    this.registerQRCodeHandlers()
    this.registerTagHandlers()
    this.registerDiagnosticHandlers()
  }

  /**
   * 剪贴板历史相关 handler
   */
  private registerHistoryHandlers(): void {
    this.registerHandler(
      'clipboard-get-history',
      async (
        _,
        options?: {
          limit?: number
          offset?: number
          tagIds?: string[]
          parseType?: string
          searchText?: string
          orderBy?: 'timestamp' | 'last_used'
          orderDirection?: 'ASC' | 'DESC'
        }
      ) => {
        const result = await clipboardHistoryService.searchHistory(options || {})
        return {
          success: true,
          data: result.items,
          apps: result.apps
        }
      },
      '获取剪贴板历史'
    )

    this.registerHandler(
      'clipboard-delete-item',
      async (_, itemId: string) => {
        await clipboardTagService.unbindAllTagsFromRecord(itemId)

        const success = await clipboardHistoryService.deleteHistoryItem(itemId)
        if (success) {
          await clipboardManager.syncDeletedRecords([itemId])
          this.notifyStateChange({ history: true, tags: true })
        }

        return {
          success,
          message: success ? '剪贴板项已删除' : '删除失败，未找到指定项目'
        }
      },
      '删除剪贴板项'
    )

    this.registerHandler(
      'clipboard-clear-history',
      async () => {
        await clipboardHistoryService.clearHistory()
        await clipboardManager.syncDeletedRecords(['*'])
        this.notifyStateChange({ history: true, tags: true })

        return {
          success: true,
          message: '剪贴板历史已清空'
        }
      },
      '清空剪贴板历史'
    )

    this.registerHandler(
      'clipboard-get-status',
      async () => {
        const isListening = await clipboardManager.isListeningActive()
        const config = await clipboardManager.getConfig()
        const historyResult = await clipboardHistoryService.searchHistory({ limit: 1 })
        const historyCount = historyResult.items.length

        return {
          success: true,
          data: {
            isListening,
            config,
            historyCount
          }
        }
      },
      '获取剪贴板状态'
    )
  }

  /**
   * 剪贴板控制相关 handler
   */
  private registerClipboardControlHandlers(): void {
    const workerPassThroughHandlers = [
      {
        channel: 'clipboard-restore',
        method: 'restoreClipboardFromDatabase',
        context: '还原剪贴板'
      },
      {
        channel: 'clipboard-restore-as-text',
        method: 'restoreClipboardFromDatabaseAsText',
        context: '还原剪贴板为纯文本'
      },
      {
        channel: 'clipboard-restore-ocr-text',
        method: 'restoreClipboardFromDatabaseAsOcrText',
        context: '还原剪贴板为OCR文本'
      }
    ] as const

    workerPassThroughHandlers.forEach(({ channel, method, context }) => {
      this.registerHandler(
        channel,
        async (_, itemId: string) => clipboardManager.call(method, itemId),
        context
      )
    })

    const listenerToggleHandlers = [
      {
        channel: 'clipboard-start-listening',
        action: () => clipboardManager.startListening(),
        message: '剪贴板监听已启动',
        context: '启动剪贴板监听'
      },
      {
        channel: 'clipboard-stop-listening',
        action: () => clipboardManager.stopListening(),
        message: '剪贴板监听已停止',
        context: '停止剪贴板监听'
      }
    ] as const

    listenerToggleHandlers.forEach(({ channel, action, message, context }) => {
      this.registerHandler(
        channel,
        async () => {
          await action()
          return {
            success: true,
            message
          }
        },
        context
      )
    })

    this.registerHandler(
      'clipboard-update-config',
      async (_, config: Partial<ClipboardManagerConfig>) => {
        await clipboardManager.updateConfig(config)
        return {
          success: true,
          message: '剪贴板配置已更新'
        }
      },
      '更新剪贴板配置'
    )
  }

  /**
   * 二维码相关 handler
   */
  private registerQRCodeHandlers(): void {
    this.registerHandler(
      'clipboard-parse-qrcode',
      async (_, itemId: string) => clipboardManager.call('parseQRCodeFromItem', itemId),
      '解析二维码'
    )

    this.registerHandler(
      'clipboard-generate-qrcode',
      async (_, itemId: string) => {
        const historyItem = await clipboardHistoryService.getHistoryItemById(itemId)
        if (!historyItem) {
          return {
            success: false,
            error: '未找到指定的剪贴板项目'
          }
        }

        const { item } = historyItem
        if (!['text', 'html', 'url'].includes(item.primaryType)) {
          return {
            success: false,
            error: '只有文本类型的内容才能生成二维码'
          }
        }

        const content = item.content as { text?: string; html?: string }
        let textContent = content.text || ''
        if (!textContent && content.html) {
          textContent = content.html.replace(/<[^>]*>/g, '')
        }

        if (!textContent.trim()) {
          return {
            success: false,
            error: '文本内容为空，无法生成二维码'
          }
        }

        const qrCodeBuffer = await qrCodeService.generateQRCodeBuffer(textContent)
        const success = await clipboardManager.call('setImageToClipboard', qrCodeBuffer)

        if (!success) {
          return {
            success: false,
            error: '设置二维码到剪贴板失败'
          }
        }

        return {
          success: true,
          message: '二维码已生成并复制到剪贴板'
        }
      },
      '生成二维码'
    )
  }

  /**
   * 标签相关 handler
   */
  private registerTagHandlers(): void {
    this.registerHandler(
      'get-all-tags',
      async () => {
        const tags = await clipboardTagService.getAllTagGroups()
        return {
          success: true,
          data: tags
        }
      },
      '获取所有标签'
    )

    this.registerHandler(
      'create-tag',
      async (_, data: { name: string; color: string; description?: string }) => {
        const tagId = await clipboardTagService.createTagGroup(
          data.name,
          data.color,
          data.description
        )
        if (!tagId) {
          return {
            success: false,
            error: '创建标签失败'
          }
        }

        this.notifyTagsUpdated()
        return {
          success: true,
          data: { tagId },
          message: '标签创建成功'
        }
      },
      '创建标签'
    )

    this.registerHandler(
      'update-tag',
      async (
        _,
        data: { tagId: string; updates: { name?: string; color?: string; description?: string } }
      ) => {
        const success = await clipboardTagService.updateTagGroup(data.tagId, data.updates)
        if (success) {
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签更新成功' : '更新标签失败'
        }
      },
      '更新标签'
    )

    this.registerHandler(
      'delete-tag',
      async (_, tagId: string) => {
        const success = await clipboardTagService.deleteTagGroup(tagId)
        if (success) {
          this.notifyTagsUpdated()
        }
        return {
          success,
          message: success ? '标签删除成功' : '删除标签失败'
        }
      },
      '删除标签'
    )

    this.registerHandler(
      'get-tag-stats',
      async () => {
        const stats = await clipboardTagService.getTagStats()
        return {
          success: true,
          data: stats
        }
      },
      '获取标签统计信息'
    )

    this.registerHandler(
      'bind-tag-to-record',
      async (_, data: { contentId: string; tagId: string }) => {
        const success = await clipboardTagService.bindTagToRecord(data.contentId, data.tagId)
        if (success) {
          this.notifyStateChange({ history: true, tags: true })
        }
        return {
          success,
          message: success ? '标签绑定成功' : '绑定标签失败'
        }
      },
      '绑定标签'
    )

    this.registerHandler(
      'unbind-tag-from-record',
      async (_, data: { contentId: string; tagId: string }) => {
        const success = await clipboardTagService.unbindTagFromRecord(data.contentId, data.tagId)
        if (success) {
          this.notifyStateChange({ history: true, tags: true })
        }
        return {
          success,
          message: success ? '标签解绑成功' : '解除标签绑定失败'
        }
      },
      '解除标签绑定'
    )
  }

  /**
   * 诊断相关 handler
   */
  private registerDiagnosticHandlers(): void {
    this.registerHandler(
      'export-heapdump',
      async () => {
        const timestamp = new Date().toISOString().replace(/[:.]/g, '-')
        const desktopPath = app.getPath('desktop')
        const exportedFiles: string[] = []

        const mainFilename = `heapdump-main-${timestamp}.heapsnapshot`
        const mainFilepath = join(desktopPath, mainFilename)
        const actualMainFilepath = writeHeapSnapshot(mainFilepath)
        exportedFiles.push(actualMainFilepath)
        logger.info('[IPC管理器] 主进程 heapdump 导出成功:', actualMainFilepath)

        const workers = this.getAllWorkerManagers()
        await Promise.all(
          workers.map(async (worker) => {
            try {
              const workerFilepath = await worker.exportWorkerHeapdump(desktopPath)
              if (workerFilepath) {
                exportedFiles.push(workerFilepath)
              }
            } catch (error) {
              logger.error('[IPC管理器] Worker heapdump 导出失败:', error)
            }
          })
        )

        const message =
          exportedFiles.length > 1
            ? `内存快照已导出到桌面 (${exportedFiles.length} 个文件)`
            : `内存快照已导出到桌面: ${mainFilename}`

        return {
          success: true,
          message,
          files: exportedFiles,
          mainFile: actualMainFilepath
        }
      },
      '导出 heapdump'
    )
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
      'clipboard-restore-ocr-text',
      'clipboard-parse-qrcode',
      'clipboard-generate-qrcode',
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
      'unbind-tag-from-record',
      'export-heapdump'
    ]

    clipboardChannels.forEach((channel) => {
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

  /**
   * 获取所有活跃的 Worker 管理器
   */
  private getAllWorkerManagers(): BaseWorkerManager[] {
    const workers: BaseWorkerManager[] = []

    // 添加剪贴板管理器
    if (clipboardManager.isRunning()) {
      workers.push(clipboardManager)
    }

    // 清理器已优化为独立 Worker，由 index.ts 管理
    // 如需导出清理器的 heapdump，需要在 index.ts 中暴露清理器实例

    return workers
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
