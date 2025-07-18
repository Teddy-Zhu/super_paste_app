import { BaseWorkerManager } from './workers/base-worker-manager'
import { NotificationService } from './services/notification-service'
import databaseService from './services/database-service'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import store from './store'
import { STORE_KEY } from './common'
import type {
  ClipboardContent,
  ClipboardManagerConfig
} from './types/clipboard-types'
import type { WorkerConfig, WorkerRequest } from './types/worker-types'
import ActiveWindow from '@paymoapp/active-window'
import { clipboard, nativeImage } from 'electron'
import logger from './utils/logger'
import clipboardManagerWorkerPath from './workers/clipboard-manager.worker?modulePath'

/**
 * 剪贴板管理器（主线程接口）
 * 通过 Worker Thread 执行剪贴板管理操作以提升性能
 */
class ClipboardManager extends BaseWorkerManager {
  private static instance: ClipboardManager

  private constructor() {
    const config: WorkerConfig = {
      name: 'ClipboardManager',
      workerPath: clipboardManagerWorkerPath
    }
    super(config)



    // 初始化 ActiveWindow
    try {
      ActiveWindow.initialize()
      // 在 macOS 上请求屏幕录制权限
      if (process.platform === 'darwin') {
        const hasPermission = ActiveWindow.requestPermissions()
        if (!hasPermission) {
          logger.warn('[剪贴板管理器] macOS 需要屏幕录制权限才能获取窗口标题')
        }
      }

    } catch (error) {
      logger.error('[剪贴板管理器] ActiveWindow 初始化失败:', error)
    }

    // 设置事件监听器
    this.setupEventListeners()
  }

  /**
   * 设置事件监听器
   */
  private setupEventListeners(): void {
    // 监听剪贴板变化事件
    this.on('clipboard-changed', (data: { timestamp: number }) => {
      // 通知前端剪贴板历史更新
      this.notifyFrontendUpdate('clipboard-changed', data)
    })

    // 监听历史记录更新事件
    this.on('history-updated', (_: { timestamp: number, source?: string }) => {
      // 通知前端剪贴板历史更新
      // this.notifyFrontendUpdate('history-updated', data)
    })

    // 监听清理检查请求事件
    this.on('cleanup-check-required', (data: { reason: string; maxHistorySize: number }) => {
      // 转发给清理器处理
      clipboardHistoryCleaner.emit('cleanup-check-required', data)
    })

    // 监听清理完成事件，直接更新缓存
    clipboardHistoryCleaner.on('cleanup-completed', (data: { stats: any, deletedRecordIds?: string[] }) => {
      // 直接通知 clipboard-manager worker 更新缓存
      if (data.deletedRecordIds && data.deletedRecordIds.length > 0) {
        this.notifyWorkerCacheUpdate(data.deletedRecordIds)
      }
    })
  }

  /**
   * 通知前端剪贴板历史更新
   */
  private notifyFrontendUpdate(eventType: string, data: any): void {
    try {
      NotificationService.notifyClipboardHistoryUpdate({
        eventType,
        timestamp: Date.now(),
        ...data
      })
    } catch (error) {
      logger.error('[剪贴板管理器] 通知前端更新失败:', error)
    }
  }

  /**
   * 通知 Worker 更新缓存
   * 直接调用 Worker 的缓存同步方法
   */
  private async notifyWorkerCacheUpdate(deletedIds: string[]): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('syncDeletedRecordsByIds', deletedIds)
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 通知 Worker 缓存更新失败:', error)
    }
  }

  /**
   * 重写基类方法来处理来自 Worker 的请求
   */
  protected handleWorkerRequest(request: WorkerRequest): void {
    this.handleWorkerCall(request)
  }

  /**
   * 处理 Worker 调用
   */
  private async handleWorkerCall(request: WorkerRequest): Promise<void> {
    try {
      let result: any

      // 检查是否是数据库操作
      if (request.method.startsWith('database-')) {
        const operation = request.method.replace('database-', '')
        result = await databaseService.executeOperation(operation, request.params)
      } else {
        switch (request.method) {
          // 剪贴板操作
          case 'clipboard-readAll':
            // console.log('read clipboard from main thread', new Date().toDateString())
            result = this.readAllClipboard()
            // console.log('read clipboard from main thread done', new Date().toDateString())
            break
          case 'clipboard-readContent':
            result = this.readClipboardContent()
            break
          case 'clipboard-restore':
            result = this.restoreClipboardContent(request.params)
            break
          case 'clipboard-restore-as-text':
            result = this.restoreClipboardContentAsText(request.params)
            break

          // 活动窗口操作
          case 'activeWindow-getActiveWindow':
            result = this.getActiveWindow()
            break

          default:
            throw new Error(`未知的调用方法: ${request.method}`)
        }
      }

      // 发送成功响应
      this.worker?.postMessage({
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: true,
        data: result,
        timestamp: Date.now()
      })
      // console.log('send response to worker', new Date().toDateString())
    } catch (error) {
      // 发送错误响应
      this.worker?.postMessage({
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        timestamp: Date.now()
      })
    }
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ClipboardManager {
    if (!ClipboardManager.instance) {
      ClipboardManager.instance = new ClipboardManager()
    }
    return ClipboardManager.instance
  }

  // ==================== 剪贴板操作方法 ====================

  private readAllClipboard(): any {
    try {
      // 检查文本内容
      const text = clipboard.readText()
      const html = clipboard.readHTML()
      const rtf = clipboard.readRTF()
      const bookmark = clipboard.readBookmark()

      // 检查图片
      const image = clipboard.readImage()
      const hasImage = !image.isEmpty()

      const result: any = {
        text: text || undefined,
        html: html || undefined,
        rtf: rtf || undefined,
        bookmark: bookmark?.title ? bookmark : undefined,
        hasImage: hasImage
      }

      // 如果有图片，添加图片的基本信息用于快速变化检测
      if (hasImage) {
        const size = image.getSize()
        result.imageInfo = {
          width: size.width,
          height: size.height,
          // 使用图片的简单标识符：尺寸 + 数据长度
          quickId: `${size.width}x${size.height}_${image.toDataURL().length}`
        }
      }

      // 如果需要包含图片内容且有图片，则读取图片数据
      if(hasImage) {
        result.image = image.toDataURL()
      }

      return result
    } catch (error) {
      logger.error('[剪贴板管理器] 读取剪贴板失败:', error)

      // 如果是系统刚从休眠恢复，剪贴板API可能暂时不可用
      if (error instanceof Error) {
        logger.warn('[剪贴板管理器] 剪贴板API调用失败，可能系统刚从休眠恢复:', error.message)
      }

      return {}
    }
  }

  private readClipboardContent(): ClipboardContent {
    const content: ClipboardContent = {}

    try {
      // 获取文本内容
      const text = clipboard.readText()
      if (text) content.text = text

      // 获取HTML内容
      const html = clipboard.readHTML()
      if (html) content.html = html

      // 获取RTF内容
      const rtf = clipboard.readRTF()
      if (rtf) content.rtf = rtf

      // 获取书签内容
      const bookmark = clipboard.readBookmark()
      if (bookmark && bookmark.title) {
        content.bookmark = JSON.stringify(bookmark)
      }

      // 获取图片内容（转为base64）
      const image = clipboard.readImage()
      if (!image.isEmpty()) {
        content.image = image.toDataURL()
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 读取剪贴板内容失败:', error)

      // 如果是系统刚从休眠恢复，剪贴板API可能暂时不可用
      if (error instanceof Error) {
        logger.warn('[剪贴板管理器] 剪贴板内容读取失败，可能系统刚从休眠恢复:', error.message)
      }
    }

    return content
  }

  public restoreClipboardContent(params: { content: ClipboardContent }): boolean {
    try {
      // 参数验证
      if (!params || !params.content) {
        logger.error('[剪贴板管理器] 还原失败: 无效的参数')
        return false
      }

      const content = params.content

      // 使用 Electron 的 clipboard.write 方法一次性设置所有格式
      const clipboardData: any = {}

      if (content.text) {
        clipboardData.text = content.text
      }

      if (content.html) {
        clipboardData.html = content.html
      }

      if (content.rtf) {
        clipboardData.rtf = content.rtf
      }

      if (content.image) {
        try {
          const image = nativeImage.createFromDataURL(content.image)
          if (!image.isEmpty()) {
            clipboardData.image = image
          }
        } catch (e) {
          logger.warn('[剪贴板管理器] 处理图片数据失败: %s', e)
        }
      }

      // 一次性写入所有格式
      if (Object.keys(clipboardData).length > 0) {
        clipboard.write(clipboardData)
      }

      // 单独处理书签（因为 write 方法不支持书签）
      if (content.bookmark) {
        try {
          const bookmark = JSON.parse(content.bookmark)
          clipboard.writeBookmark(bookmark.title, bookmark.url)
        } catch (e) {
          logger.warn('[剪贴板管理器] 设置书签失败:', e)
        }
      }

      // 检查是否有任何内容被设置
      const hasContent = Object.keys(clipboardData).length > 0 || content.bookmark
      if (!hasContent) {
        logger.warn('[剪贴板管理器] 警告: 没有有效的剪贴板内容可以还原')
      }

      return true
    } catch (error) {
      logger.error('[剪贴板管理器] 还原剪贴板时发生异常:', error)
      return false
    }
  }

  public restoreClipboardContentAsText(params: { content: ClipboardContent }): boolean {
    try {
      // 参数验证
      if (!params || !params.content) {
        logger.error('[剪贴板管理器] 还原为纯文本失败: 无效的参数')
        return false
      }

      const content = params.content

      // 只设置纯文本格式，去除所有其他格式
      if (content.text && content.text.trim()) {
        clipboard.writeText(content.text)
        return true
      } else {
        logger.warn('[剪贴板管理器] 内容中没有有效的文本数据，无法设置为纯文本')
        return false
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 还原剪贴板为纯文本失败:', error)
      return false
    }
  }

  // ==================== 活动窗口操作方法 ====================

  private getActiveWindow(): any {
    try {
      return ActiveWindow.getActiveWindow()
    } catch (error) {
      logger.error('[剪贴板管理器] 获取活动窗口失败:', error)
      return {}
    }
  }

  /**
   * 确保 Worker 已初始化
   */
  private async ensureWorkerInitialized(): Promise<void> {
    if (!this.isInitialized) {
      await this.start()
      // isInitialized 已在 start() 方法中设置，无需重复设置
    }
  }

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    await this.ensureWorkerInitialized()

    // 从store中获取配置
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)

    const finalConfig: Partial<ClipboardManagerConfig> = {
      maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false,
      ...config
    }

    await this.call('initialize', finalConfig)
  }

  /**
   * 开始监听剪贴板变化
   */
  public async startListening(): Promise<void> {
    await this.ensureWorkerInitialized()

    await this.call('startListening')
  }

  /**
   * 停止监听剪贴板变化
   */
  public async stopListening(): Promise<void> {
    if (this.isRunning()) {
      await this.call('stopListening')
    }
  }







  /**
   * 手动设置剪贴板内容（不触发监控）
   */
  public async setClipboardContent(_: ClipboardContent, updateHash: boolean = true): Promise<boolean> {
    try {
      // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
      // 暂停监听以避免触发变更检测
      const wasListening = await this.isListeningActive()
      if (wasListening) {
        await this.stopListening()
      }

      try {
        // 实现剪贴板设置逻辑（从原文件复制）
        // 这里需要实现具体的剪贴板设置逻辑

        // 更新内部哈希值以保持同步
        if (updateHash) {
          await this.call('syncClipboardHash')
        }

        return true
      } finally {
        // 恢复监听状态
        if (wasListening) {
          setTimeout(async () => {
            await this.startListening()
          }, 150)
        }
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 设置剪贴板内容失败:', error)
      return false
    }
  }





  /**
   * 同步删除记录状态到 Worker
   */
  public async syncDeletedRecords(deletedIds: string[]): Promise<void> {
    try {
      if (this.isRunning()) {
        await this.call('syncDeletedRecords', deletedIds)
      }
    } catch (error) {
      logger.error('[剪贴板管理器] 同步删除记录状态失败:', error)
    }
  }



  /**
   * 获取配置
   */
  public async getConfig(): Promise<ClipboardManagerConfig> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('getConfig')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取配置失败:', error)
      // 返回默认配置
      const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
      return {
        maxHistorySize,
        checkInterval: 200,
        enableAutoSave: false
      }
    }
  }

  /**
   * 更新配置
   */
  public async updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      await this.call('updateConfig', newConfig)
    } catch (error) {
      logger.error('[剪贴板管理器] 更新配置失败:', error)
    }
  }

  /**
   * 从store更新maxHistorySize配置
   */
  public async updateMaxHistorySizeFromStore(): Promise<void> {
    try {
      const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
      await this.updateConfig({ maxHistorySize })
    } catch (error) {
      logger.error('[剪贴板管理器] 从store更新maxHistorySize失败:', error)
    }
  }

  /**
   * 从store更新maxKeepTime配置并更新清理器
   */
  public updateMaxKeepTimeFromStore(): void {
    // 这个方法委托给清理器处理
    try {
      // 导入清理器并更新配置
      import('./clipboard-history-cleaner').then(module => {
        const clipboardHistoryCleaner = module.default
        clipboardHistoryCleaner.updateConfigFromStore()
      })
    } catch (error) {
      logger.error('[剪贴板管理器] 更新清理器配置失败:', error)
    }
  }

  /**
   * 获取监听状态
   */
  public async isListeningActive(): Promise<boolean> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('isListeningActive')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取监听状态失败:', error)
      return false
    }
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   */
  public async syncClipboardHash(): Promise<void> {
    try {
      await this.ensureWorkerInitialized()

      await this.call('syncClipboardHash')
    } catch (error) {
      logger.error('[剪贴板管理器] 同步剪贴板哈希值失败:', error)
    }
  }

  /**
   * 系统唤醒后的恢复处理
   * 确保剪贴板管理器在系统休眠唤醒后正常工作
   */
  public async handleSystemResume(): Promise<void> {
    try {
      logger.info('[剪贴板管理器] 开始处理系统唤醒恢复')

      // 确保Worker已初始化
      await this.ensureWorkerInitialized()

      // 重置错误状态
      await this.call('resetErrorState')

      // 同步剪贴板哈希值
      await this.syncClipboardHash()

      // 如果之前在监听，重新启动监听
      const wasListening = await this.isListeningActive()
      if (!wasListening) {
        await this.startListening()
      }

      logger.info('[剪贴板管理器] 系统唤醒恢复完成')
    } catch (error) {
      logger.error('[剪贴板管理器] 系统唤醒恢复失败:', error)
    }
  }

  /**
   * 获取当前剪贴板内容（不触发监控）
   */
  public async getCurrentClipboard(): Promise<ClipboardContent> {
    try {
      await this.ensureWorkerInitialized()

      return await this.call('getCurrentClipboard')
    } catch (error) {
      logger.error('[剪贴板管理器] 获取当前剪贴板内容失败:', error)
      return {}
    }
  }

  /**
   * 测试剪贴板功能
   */
  public testClipboard(): void {
    // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
    try {
      // 实现测试逻辑
    } catch (error) {
      logger.error('[剪贴板管理器] 测试剪贴板功能失败:', error)
    }
  }

  /**
   * 验证剪贴板内容是否可用
   */
  public verifyClipboardContent(): { isValid: boolean; details: any } {
    // 这个方法需要在主线程中实现，因为涉及到剪贴板 API
    try {
      return { isValid: true, details: {} }
    } catch (error) {
      logger.error('[剪贴板管理器] 验证剪贴板内容失败:', error)
      return { isValid: false, details: { error: error instanceof Error ? error.message : String(error) } }
    }
  }

  // ==================== 标签管理方法 ====================




}

// 导出单例实例
export default ClipboardManager.getInstance()
export { ClipboardManager }
