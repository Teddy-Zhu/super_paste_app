import { ipcMain, app, BrowserWindow, shell } from 'electron'
import type { IpcMainInvokeEvent } from 'electron'
import clipboardManager from '../features/clipboard/manager/clipboard-manager'
import clipboardHistoryService from '../services/clipboard-history-service'
import clipboardTagService from '../services/clipboard-tag-service'
import { qrCodeService } from '../services/qrcode-service'
import { NotificationService } from '../services/notification-service'
import type {
  ClipboardItem,
  ClipboardManagerConfig
} from '../features/clipboard/types/clipboard-types'
import type { BaseWorkerManager } from '../workers/base-worker-manager'
import { mainWindowManager, overlayWindowManager } from '../ui/windows/window-managers'
import { globalShortcutManager } from '../system/global-shortcut-manager'
import { isWayland } from '../system/platform'
import { activeWindowService } from '../services/active-window-service'
import autoStartManager from '../system/auto-start-manager'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import { debounceExecution } from '../utils/common'
import { MAX_HISTORY_SIZE_DEBOUNCE_DELAY, MAX_KEEP_TIME_DEBOUNCE_DELAY } from '../config/constants'
import { scheduledSyncManager } from '../sync/scheduled-sync-manager'
import type { ScheduledSyncConfig } from '../sync/types'
import { SyncStatus } from '../sync/types'
import { SyncConfigUtils } from '../sync/sync-config-utils'
import { trayManager } from '../ui/tray/tray-manager'
import logger from '../utils/logger'
import { writeHeapSnapshot } from 'v8'
import { join } from 'path'
import type { SystemAccessStatus } from '../../shared/system-access'
import { DEFAULT_OCR_MODEL_ID, type OCRSettings } from '../../shared/ocr-models'
import { getOCRModelStore } from '../ocr/model-service'
import { OCR_SETTING_KEYS, parseOCRSettings, toClipboardOCRConfig } from '../ocr/settings'

/**
 * IPC 管理器
 * 统一注册和管理所有 IPC 事件处理器
 */
class IpcManager {
  private static instance: IpcManager
  private isInitialized: boolean = false
  private ocrSettingsQueue: Promise<unknown> = Promise.resolve()

  private constructor() {
    // 通过静态方法统一创建单例。
  }

  public static getInstance(): IpcManager {
    if (!IpcManager.instance) {
      IpcManager.instance = new IpcManager()
    }
    return IpcManager.instance
  }

  // ==================== 工具方法 ====================

  /**
   * 通用 IPC handler 注册工具（invoke/handle 模式）
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
   * 按需通知前端状态变化
   */
  private notifyStateChange(options: { history?: boolean; tags?: boolean }): void {
    const { history, tags } = options
    if (history) {
      NotificationService.notifyClipboardHistoryUpdate({ timestamp: Date.now() })
    }
    if (tags) {
      NotificationService.notifyTagsUpdate({ timestamp: Date.now() })
    }
  }

  // ==================== 初始化 / 清理 ====================

  /**
   * 初始化所有 IPC 处理器
   */
  public initialize(): void {
    if (this.isInitialized) {
      logger.debug('[IPC管理器] 已初始化，跳过重复初始化')
      return
    }

    // 初始化前先清理可能存在的旧处理器
    this.cleanup()

    logger.debug('[主进程] 开始注册 IPC 事件处理器')

    this.registerClipboardHandlers()
    this.registerWindowHandlers()
    this.registerStoreHandlers()
    this.registerAutoStartHandlers()
    this.registerAnimationHandlers()
    this.registerScheduledSyncHandlers()
    this.registerSystemHandlers()
    this.registerOCRHandlers()

    this.isInitialized = true
    logger.debug('[IPC管理器] 所有 IPC 事件处理器注册完成')
  }

  /**
   * 清理所有 IPC 处理器
   */
  public cleanup(): void {
    if (!this.isInitialized) {
      return
    }

    const channels = [
      // 剪贴板历史
      'clipboard-get-history',
      'clipboard-restore',
      'clipboard-restore-as-text',
      'clipboard-restore-ocr-text',
      'clipboard-parse-ocr',
      'clipboard-parse-qrcode',
      'clipboard-generate-qrcode',
      'clipboard-delete-item',
      'clipboard-clear-history',
      'clipboard-get-status',
      'clipboard-start-listening',
      'clipboard-stop-listening',
      'clipboard-update-config',
      // 标签
      'get-all-tags',
      'create-tag',
      'update-tag',
      'delete-tag',
      'get-tag-stats',
      'bind-tag-to-record',
      'unbind-tag-from-record',
      // 诊断
      'export-heapdump',
      // 自启动
      'auto-start-get-status',
      'auto-start-set-status',
      'auto-start-toggle',
      // 存储
      'store-get',
      'store-set',
      // 定时同步
      'scheduled-sync:initialize',
      'scheduled-sync:start',
      'scheduled-sync:stop',
      'scheduled-sync:manual-sync',
      'scheduled-sync:manual-sync-with-config',
      'scheduled-sync:test-connection',
      'scheduled-sync:get-status',
      'scheduled-sync:update-config',
      'scheduled-sync:refresh',
      // OCR 模型
      'ocr:list-models',
      'ocr:download-model',
      'ocr:cancel-download',
      'ocr:save-settings',
      // 系统
      'system-access-status',
      'system-access-retry',
      'app:restart',
      'app:window-minimize',
      'app:window-close'
    ]

    channels.forEach((channel) => {
      ipcMain.removeHandler(channel)
    })
    for (const channel of [
      'open-overlay-window',
      'close-overlay-window',
      'overlay-ready',
      'open-overlay-finished',
      'overlay-open-prepared',
      'overlay-close-started',
      'overlay-trace'
    ]) {
      ipcMain.removeAllListeners(channel)
    }

    this.isInitialized = false
    logger.debug('[IPC管理器] 所有 IPC 事件处理器已清理')
  }

  /**
   * 获取初始化状态
   */
  public getInitializationStatus(): boolean {
    return this.isInitialized
  }

  // ==================== 剪贴板相关 ====================

  private registerClipboardHandlers(): void {
    this.registerHistoryHandlers()
    this.registerClipboardControlHandlers()
    this.registerQRCodeHandlers()
    this.registerTagHandlers()
    this.registerDiagnosticHandlers()
  }

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

  private registerQRCodeHandlers(): void {
    this.registerHandler(
      'clipboard-parse-qrcode',
      async (_, itemId: string) => clipboardManager.call('parseQRCodeFromItem', itemId),
      '解析二维码'
    )

    this.registerHandler(
      'clipboard-parse-ocr',
      async (_, itemId: string) => clipboardManager.parseOcrFromItem(itemId),
      '解析OCR'
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

        NotificationService.notifyTagsUpdate({ timestamp: Date.now(), action: 'create' })
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
          NotificationService.notifyTagsUpdate({ timestamp: Date.now(), action: 'update' })
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
          NotificationService.notifyTagsUpdate({ timestamp: Date.now(), action: 'delete' })
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
   * 获取所有活跃的 Worker 管理器
   */
  private getAllWorkerManagers(): BaseWorkerManager[] {
    const workers: BaseWorkerManager[] = []

    if (clipboardManager.isRunning()) {
      workers.push(clipboardManager)
    }

    return workers
  }

  // ==================== 窗口相关 ====================

  private registerWindowHandlers(): void {
    // 打开 overlay 窗口
    ipcMain.on('open-overlay-window', () => {
      try {
        if (overlayWindowManager.isValid()) {
          overlayWindowManager.toggleWindow()
        } else {
          overlayWindowManager.createWindow()
          overlayWindowManager.showWindow()
        }
      } catch (error) {
        logger.error('[主进程] 打开 Overlay 窗口失败:', error)
      }
    })

    // 关闭 overlay 窗口
    ipcMain.on('close-overlay-window', (event, id?: number) => {
      if (event.sender !== overlayWindowManager.getWindow()?.webContents) return
      try {
        if (typeof id === 'number' && Number.isInteger(id)) overlayWindowManager.finishClosing(id)
      } catch (error) {
        logger.error('[主进程] 关闭 Overlay 窗口失败:', error)
      }
    })

    // 打开主窗口
    ipcMain.on('open-main-window', () => {
      try {
        mainWindowManager.showWindow()
      } catch (error) {
        logger.error('[主进程] 打开主窗口失败:', error)
      }
    })

    // 在系统默认浏览器中打开链接
    ipcMain.on('open-external-url', (_, url: string) => {
      try {
        const targetUrl = (url || '').trim()
        if (!targetUrl) return

        const parsed = new URL(targetUrl)
        if (!['http:', 'https:'].includes(parsed.protocol)) {
          logger.warn('[主进程] 拒绝打开非 http(s) 链接:', targetUrl)
          return
        }

        shell.openExternal(parsed.toString()).catch((error) => {
          logger.error('[主进程] 打开外部链接失败:', error)
        })
      } catch (error) {
        logger.error('[主进程] 解析外部链接失败:', error)
      }
    })

    this.registerHandler(
      'app:window-minimize',
      async (event) => {
        const targetWindow = BrowserWindow.fromWebContents(event.sender)
        if (!targetWindow || targetWindow.isDestroyed()) {
          return { success: false, error: '当前窗口不可用' }
        }

        if (targetWindow.minimizable) {
          targetWindow.minimize()
        } else {
          targetWindow.hide()
        }

        return { success: true }
      },
      '最小化当前窗口'
    )

    this.registerHandler(
      'app:window-close',
      async (event) => {
        const targetWindow = BrowserWindow.fromWebContents(event.sender)
        if (!targetWindow || targetWindow.isDestroyed()) {
          return { success: false, error: '当前窗口不可用' }
        }

        targetWindow.close()
        return { success: true }
      },
      '关闭当前窗口'
    )
  }

  private registerOCRHandlers(): void {
    this.registerHandler(
      'ocr:list-models',
      async () => ({ success: true, data: await getOCRModelStore().list() }),
      '读取 OCR 模型'
    )
    this.registerHandler(
      'ocr:download-model',
      async (_, id: unknown) => {
        await getOCRModelStore().download(id)
        return { success: true }
      },
      '下载 OCR 模型'
    )
    this.registerHandler(
      'ocr:cancel-download',
      async (_, id: unknown) => {
        getOCRModelStore().cancel(id)
        return { success: true }
      },
      '取消模型下载'
    )
    this.registerHandler(
      'ocr:save-settings',
      async (_, value: unknown) => {
        const settings = parseOCRSettings(value)
        const save = this.ocrSettingsQueue.then(() => this.saveOCRSettings(settings))
        this.ocrSettingsQueue = save.then(
          () => undefined,
          () => undefined
        )
        return save
      },
      '保存 OCR 配置'
    )
  }

  private async saveOCRSettings(
    settings: OCRSettings
  ): Promise<{ success: true; data: OCRSettings }> {
    const previous = await clipboardManager.getConfig()
    await clipboardManager.updateConfig(toClipboardOCRConfig(settings))
    const values = Object.fromEntries(
      Object.entries(OCR_SETTING_KEYS).map(([field, key]) => [
        key,
        settings[field as keyof OCRSettings]
      ])
    )
    try {
      // 一次写入完整配置，避免各字段触发多轮模型初始化。
      store.getStore().set(values)
    } catch (error) {
      try {
        await clipboardManager.updateConfig(previous)
      } catch (rollbackError) {
        logger.error('[OCR] 恢复配置失败:', rollbackError)
      }
      throw error
    }
    for (const [key, value] of Object.entries(values)) {
      NotificationService.notifyFrontend('store-value-changed', {
        key,
        value,
        timestamp: Date.now()
      })
    }
    trayManager.updateMenu()
    return { success: true, data: settings }
  }

  // ==================== 存储相关 ====================

  private registerStoreHandlers(): void {
    // 获取存储值（异步）
    this.registerHandler(
      'store-get',
      async (_, key: string, defaultValue?: unknown) => {
        return { success: true, data: store.get(key, defaultValue) }
      },
      '获取存储值'
    )

    // 同步获取存储值
    ipcMain.on('store-get-on', (event, key) => {
      try {
        const val = store?.get(key)
        event.returnValue = val
      } catch (error) {
        logger.error(`[主进程] 同步获取存储值失败 [${key}]:`, error)
        event.returnValue = undefined
      }
    })

    // 设置存储值
    this.registerHandler(
      'store-set',
      async (_, key: string, value: unknown) => {
        if (key === STORE_KEY.SETTINGS_GLOBAL_SHORTCUT) {
          if (typeof value !== 'string') throw new Error('快捷键必须是字符串')
          if (
            !globalShortcutManager.registerGlobalShortcut(value, () =>
              overlayWindowManager.toggleWindow(true, 'global-shortcut')
            )
          ) {
            throw new Error(globalShortcutManager.getStatus().error || '快捷键注册失败')
          }
        }
        store.raw_set(key, value)

        await this.handleSpecialConfigUpdates(key, value)

        return { success: true }
      },
      '设置存储值'
    )
  }

  /**
   * 处理特殊配置更新（全局快捷键、历史大小、OCR、自启动等）
   */
  private async handleSpecialConfigUpdates(key: string, value: unknown): Promise<void> {
    try {
      if (key === STORE_KEY.SETTINGS_MAX_HISTORY_SIZE) {
        debounceExecution(
          'update-max-history-size',
          async () => {
            logger.debug('[主进程] 最大历史记录数量配置已更新，更新剪贴板管理器配置')
            try {
              const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
              await clipboardManager.updateConfig({ maxHistorySize })
            } catch (error) {
              logger.error('[主进程] 更新剪贴板管理器maxHistorySize失败:', error)
            }
          },
          MAX_HISTORY_SIZE_DEBOUNCE_DELAY
        )
      }

      if (key === STORE_KEY.SETTINGS_MAX_KEEP_TIME) {
        debounceExecution(
          'update-max-keep-time',
          async () => {
            logger.debug('[主进程] 最大保存时间配置已更新，清理器将在下次应用启动时使用新配置')
            // TODO: 如果需要动态更新清理器配置，需要在 index.ts 中暴露清理器实例的引用
          },
          MAX_KEEP_TIME_DEBOUNCE_DELAY
        )
      }

      if (key === STORE_KEY.SETTINGS_AUTO_START) {
        const autoStart = value as boolean
        const success = await autoStartManager.setEnabled(autoStart)

        if (!success) {
          logger.error('[主进程] 设置自动启动失败')
        } else {
          logger.debug(`[主进程] 自动启动状态已更新: ${autoStart}`)
        }

        trayManager.updateMenu()
      }

      if (key === STORE_KEY.SETTINGS_OVERLAY_POSITION) {
        logger.debug('[主进程] Overlay 位置配置已更新，刷新窗口边界')
        overlayWindowManager.updateWindowPositionBySettings()
      }

      const ocrSettingKeys = new Set([
        STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR,
        STORE_KEY.SETTINGS_OCR_MODE,
        STORE_KEY.SETTINGS_OCR_LOCAL_MODEL,
        STORE_KEY.SETTINGS_OCR_REMOTE_URL,
        STORE_KEY.SETTINGS_OCR_REMOTE_EXTRA_PARAMS,
        STORE_KEY.SETTINGS_OCR_REMOTE_IMAGE_FIELD_NAME,
        STORE_KEY.SETTINGS_OCR_REMOTE_REQUEST_FORMAT,
        STORE_KEY.SETTINGS_OCR_REMOTE_RESULT_PATH
      ])

      if (ocrSettingKeys.has(key)) {
        logger.debug(`[主进程] OCR配置已更新: ${key}`)

        try {
          await clipboardManager.updateConfig({
            enableLocalOCR: store.get(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false),
            ocrMode: store.get(STORE_KEY.SETTINGS_OCR_MODE, 'local'),
            ocrLocalModel: store.get(STORE_KEY.SETTINGS_OCR_LOCAL_MODEL, DEFAULT_OCR_MODEL_ID),
            ocrRemoteConfig: {
              url: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_URL, ''),
              extraParams: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_EXTRA_PARAMS, '{}'),
              imageFieldName: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_IMAGE_FIELD_NAME, 'image'),
              requestFormat: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_REQUEST_FORMAT, 'json'),
              resultPath: store.get(STORE_KEY.SETTINGS_OCR_REMOTE_RESULT_PATH, 'data.result')
            }
          })
          logger.debug('[主进程] 剪贴板管理器 OCR 配置已更新')
        } catch (error) {
          logger.error('[主进程] 更新剪贴板管理器 OCR 配置失败:', error)
        }

        if (key === STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR) {
          trayManager.updateMenu()
        }
      }

      logger.debug(`[主进程] 处理特殊配置更新完成 [${key}]`)
    } catch (error) {
      logger.error(`[主进程] 处理特殊配置更新失败 [${key}]:`, error)
    }
  }

  // ==================== 自启动相关 ====================

  private registerAutoStartHandlers(): void {
    this.registerHandler(
      'auto-start-get-status',
      async () => {
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
      },
      '获取自启动状态'
    )

    this.registerHandler(
      'auto-start-set-status',
      async (_, enabled: boolean) => {
        const success = await autoStartManager.setEnabled(enabled)
        return { success, data: { enabled } }
      },
      '设置自启动状态'
    )

    this.registerHandler(
      'auto-start-toggle',
      async () => {
        const newState = await autoStartManager.toggle()
        return { success: true, data: { enabled: newState } }
      },
      '切换自启动状态'
    )
  }

  // ==================== 动画相关 ====================

  private registerAnimationHandlers(): void {
    ipcMain.on('overlay-ready', (event) => {
      if (event.sender === overlayWindowManager.getWindow()?.webContents) {
        overlayWindowManager.markRendererReady()
      }
    })
    ipcMain.on('open-overlay-finished', (event, id: number) => {
      if (event.sender === overlayWindowManager.getWindow()?.webContents && Number.isInteger(id)) {
        overlayWindowManager.finishOpening(id)
      }
    })
    ipcMain.on('overlay-open-prepared', (event, id: number) => {
      if (
        event.sender === overlayWindowManager.getWindow()?.webContents &&
        Number.isSafeInteger(id)
      ) {
        overlayWindowManager.showPreparedWindow(id)
      }
    })
    ipcMain.on('overlay-close-started', (event, id: number) => {
      if (
        event.sender === overlayWindowManager.getWindow()?.webContents &&
        Number.isSafeInteger(id)
      ) {
        overlayWindowManager.beginClosing(id)
      }
    })
    ipcMain.on('overlay-trace', (event, data: unknown) => {
      if (event.sender === overlayWindowManager.getWindow()?.webContents) {
        overlayWindowManager.recordRendererTrace(data)
      }
    })
  }

  // ==================== 定时同步相关 ====================

  private registerScheduledSyncHandlers(): void {
    this.registerHandler(
      'scheduled-sync:initialize',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        const syncConfig: ScheduledSyncConfig = {
          ...config,
          userDataPath: app.getPath('userData')
        }
        return await scheduledSyncManager.initialize(syncConfig)
      },
      '初始化定时同步'
    )

    this.registerHandler(
      'scheduled-sync:start',
      async () => await scheduledSyncManager.startScheduledSync(),
      '启动定时同步'
    )

    this.registerHandler(
      'scheduled-sync:stop',
      async () => await scheduledSyncManager.stopScheduledSync(),
      '停止定时同步'
    )

    this.registerHandler(
      'scheduled-sync:manual-sync',
      async () => await scheduledSyncManager.manualSync(),
      '手动同步'
    )

    this.registerHandler(
      'scheduled-sync:manual-sync-with-config',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        const syncConfig: ScheduledSyncConfig = {
          ...config,
          userDataPath: app.getPath('userData')
        }
        return await scheduledSyncManager.manualSyncWithConfig(syncConfig)
      },
      '使用指定配置手动同步'
    )

    this.registerHandler(
      'scheduled-sync:test-connection',
      async (_, config?: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        if (config) {
          const syncConfig: ScheduledSyncConfig = {
            ...config,
            userDataPath: app.getPath('userData')
          }
          return await scheduledSyncManager.testConnectionWithConfig(syncConfig)
        } else {
          return await scheduledSyncManager.testConnection()
        }
      },
      '测试同步连接'
    )

    this.registerHandler(
      'scheduled-sync:get-status',
      async () => {
        if (!scheduledSyncManager.isRunning()) {
          return {
            success: true,
            data: {
              isRunning: false,
              lastSyncTime: 0,
              nextSyncTime: 0,
              syncStatus: SyncStatus.IDLE,
              config: null
            }
          }
        }

        const status = await scheduledSyncManager.getSyncStatus()
        return { success: true, data: status }
      },
      '获取同步状态'
    )

    this.registerHandler(
      'scheduled-sync:update-config',
      async (_, config: Omit<ScheduledSyncConfig, 'userDataPath'>) => {
        const syncConfig: ScheduledSyncConfig = {
          ...config,
          userDataPath: app.getPath('userData')
        }
        return await scheduledSyncManager.updateConfig(syncConfig)
      },
      '更新定时同步配置'
    )

    this.registerHandler(
      'scheduled-sync:refresh',
      async () => {
        logger.info('[主进程] 开始刷新计划同步任务')

        const syncConfig = SyncConfigUtils.loadBaseSyncConfig()

        logger.debug('[主进程] 已加载基础同步配置:', {
          enabled: syncConfig.enabled,
          provider: syncConfig.providerType,
          interval: syncConfig.syncInterval
        })

        const result = await scheduledSyncManager.updateConfig(syncConfig)

        if (result) {
          if (syncConfig.enabled) {
            logger.info('[主进程] 计划同步任务刷新成功，同步已启用')
          } else {
            logger.info('[主进程] 计划同步任务刷新成功，Worker 基础功能可用')
          }
        } else {
          logger.warn('[主进程] 计划同步任务刷新失败')
        }

        return result
      },
      '刷新计划同步任务'
    )
  }

  // ==================== 系统相关 ====================

  private registerSystemHandlers(): void {
    const getAccessStatus = async (): Promise<SystemAccessStatus> => ({
      wayland: isWayland,
      shortcut: globalShortcutManager.getStatus(),
      clipboard: await clipboardManager
        .call<SystemAccessStatus['clipboard']>('getAccessStatus')
        .catch((error) => ({
          listening: false,
          available: false,
          error: error instanceof Error ? error.message : String(error)
        })),
      sourceTracking: activeWindowService.getStatus().available,
      executable: process.execPath
    })
    this.registerHandler(
      'system-access-status',
      async () => ({ success: true, data: await getAccessStatus() }),
      '读取系统权限'
    )
    this.registerHandler(
      'system-access-retry',
      async () => {
        const shortcutRegistered = overlayWindowManager.registerGlobalShortcut(true)
        let clipboardError: string | undefined
        try {
          await clipboardManager.startListening()
        } catch (error) {
          clipboardError = error instanceof Error ? error.message : String(error)
        }
        await activeWindowService.initialize()
        return { success: true, data: await getAccessStatus(), shortcutRegistered, clipboardError }
      },
      '重试系统权限'
    )

    this.registerHandler(
      'app:restart',
      async () => {
        logger.info('[主进程] 收到应用重启请求')

        setTimeout(() => {
          app.relaunch()
          app.exit()
        }, 1000)

        return { success: true }
      },
      '重启应用'
    )
  }
}

// 导出单例实例
export default IpcManager.getInstance()
export { IpcManager }

// 导出类型定义供渲染进程使用
export interface ClipboardIpcResponse<T = unknown> {
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
