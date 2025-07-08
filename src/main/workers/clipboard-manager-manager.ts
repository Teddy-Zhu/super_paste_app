import { clipboard, nativeImage } from 'electron'
import { BaseWorkerManager } from './base-worker-manager'
import clipboardDatabase from '../clipboard-database'
import type { WorkerConfig, WorkerRequest } from '../types/worker-types'
import type { ClipboardContent, ClipboardManagerConfig, ClipboardItem } from '../types/clipboard-types'
import ActiveWindow from '@paymoapp/active-window'

/**
 * 剪贴板管理器 Worker 管理器
 * 负责管理剪贴板管理器 Worker 的生命周期和通信
 */
export class ClipboardManagerManager extends BaseWorkerManager {

  constructor(workerPath: string) {
    const config: WorkerConfig = {
      name: 'ClipboardManager',
      workerPath
    }
    super(config)

    // 监听 Worker 事件
    this.setupEventListeners()
  }

  /**
   * 设置事件监听器
   */
  private setupEventListeners(): void {
    // 监听剪贴板变化事件
    this.on('clipboard-changed', (data: { timestamp: number }) => {
      console.log('[剪贴板管理器] 剪贴板已变化:', data.timestamp)
    })

    // 监听历史记录更新事件
    this.on('history-updated', (data: { timestamp: number, source?: string }) => {
      console.log('[剪贴板管理器] 历史记录已更新:', data)
    })
  }

  /**
   * 启动 Worker 并设置调用处理
   */
  public async start(): Promise<void> {
    await super.start()
    
    // 设置调用处理
    this.setupCallHandlers()
  }

  /**
   * 设置调用处理器
   * Worker 通过这些处理器调用主线程的功能
   */
  private setupCallHandlers(): void {
    // 不需要额外的消息监听器，使用基类的 handleWorkerRequest 方法
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

      switch (request.method) {
        // 数据库操作
        case 'database-initialize':
          result = this.initializeDatabase()
          break
        case 'database-queryRecordsWithTags':
          result = this.queryRecordsWithTags(request.params)
          break
        case 'database-updateLastUsed':
          result = this.updateLastUsed(request.params)
          break
        case 'database-insertOrUpdateApp':
          result = this.insertOrUpdateApp(request.params)
          break
        case 'database-insertRecord':
          result = this.insertRecord(request.params)
          break
        case 'database-deleteRecord':
          result = this.deleteRecord(request.params)
          break
        case 'database-clearAllRecords':
          result = this.clearAllRecords()
          break
        case 'database-createTagGroup':
          result = this.createTagGroupInternal(request.params)
          break
        case 'database-getAllTagGroups':
          result = this.getAllTagGroupsInternal()
          break
        case 'database-updateTagGroup':
          result = this.updateTagGroupInternal(request.params)
          break
        case 'database-deleteTagGroup':
          result = this.deleteTagGroupInternal(request.params)
          break
        case 'database-bindTagToRecord':
          result = this.bindTagToRecordInternal(request.params)
          break
        case 'database-unbindTagFromRecord':
          result = this.unbindTagFromRecordInternal(request.params)
          break
        case 'database-getTagGroupById':
          result = this.getTagGroupById(request.params)
          break
        case 'database-getTagGroupStats':
          result = this.getTagGroupStats()
          break
        
        // 剪贴板操作
        case 'clipboard-readAll':
          result = this.readAllClipboard()
          break
        case 'clipboard-readContent':
          result = this.readClipboardContent()
          break
        case 'clipboard-restore':
          result = this.restoreClipboard(request.params)
          break
        
        // 活动窗口操作
        case 'activeWindow-getActiveWindow':
          result = this.getActiveWindow()
          break
        
        default:
          throw new Error(`未知的调用方法: ${request.method}`)
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

  // ==================== 数据库操作方法 ====================

  private initializeDatabase(): boolean {
    try {
      clipboardDatabase.initialize()
      return true
    } catch (error) {
      console.error('[剪贴板管理器] 初始化数据库失败:', error)
      return false
    }
  }

  private queryRecordsWithTags(params: any): any[] {
    try {
      return clipboardDatabase.queryRecordsWithTags(params)
    } catch (error) {
      console.error('[剪贴板管理器] 查询记录失败:', error)
      return []
    }
  }

  private updateLastUsed(params: { contentId: string, lastUsed: number }): boolean {
    try {
      return clipboardDatabase.updateLastUsed(params.contentId, params.lastUsed)
    } catch (error) {
      console.error('[剪贴板管理器] 更新最后使用时间失败:', error)
      return false
    }
  }

  private insertOrUpdateApp(params: { application: string, path: string, icon?: string }): string | null {
    try {
      return clipboardDatabase.insertOrUpdateApp(params.application, params.path, params.icon)
    } catch (error) {
      console.error('[剪贴板管理器] 插入或更新应用信息失败:', error)
      return null
    }
  }

  private insertRecord(params: any): boolean {
    try {
      return clipboardDatabase.insertRecord(
        params.contentId,
        params.content,
        params.primaryType,
        params.availableFormats,
        params.timestamp,
        params.lastUsed,
        params.parseType,
        params.sourceAppId
      )
    } catch (error) {
      console.error('[剪贴板管理器] 插入记录失败:', error)
      return false
    }
  }

  private deleteRecord(params: { contentId: string }): boolean {
    try {
      return clipboardDatabase.deleteRecord(params.contentId)
    } catch (error) {
      console.error('[剪贴板管理器] 删除记录失败:', error)
      return false
    }
  }

  private clearAllRecords(): boolean {
    try {
      clipboardDatabase.clearAllRecords()
      return true
    } catch (error) {
      console.error('[剪贴板管理器] 清空所有记录失败:', error)
      return false
    }
  }

  private createTagGroupInternal(params: { tag: any }): boolean {
    try {
      return clipboardDatabase.createTagGroup(params.tag)
    } catch (error) {
      console.error('[剪贴板管理器] 创建标签组失败:', error)
      return false
    }
  }

  private getAllTagGroupsInternal(): any[] {
    try {
      return clipboardDatabase.getAllTagGroups()
    } catch (error) {
      console.error('[剪贴板管理器] 获取所有标签组失败:', error)
      return []
    }
  }

  private updateTagGroupInternal(params: { tagId: string, updateData: any }): boolean {
    try {
      return clipboardDatabase.updateTagGroup(params.tagId, params.updateData)
    } catch (error) {
      console.error('[剪贴板管理器] 更新标签组失败:', error)
      return false
    }
  }

  private deleteTagGroupInternal(params: { tagId: string }): boolean {
    try {
      return clipboardDatabase.deleteTagGroup(params.tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 删除标签组失败:', error)
      return false
    }
  }

  private bindTagToRecordInternal(params: { contentId: string, tagId: string }): boolean {
    try {
      return clipboardDatabase.bindTagToRecord(params.contentId, params.tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 绑定标签到记录失败:', error)
      return false
    }
  }

  private unbindTagFromRecordInternal(params: { contentId: string, tagId: string }): boolean {
    try {
      return clipboardDatabase.unbindTagFromRecord(params.contentId, params.tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 解绑标签失败:', error)
      return false
    }
  }

  private getTagGroupById(params: { tagId: string }): any | null {
    try {
      return clipboardDatabase.getTagGroupById(params.tagId)
    } catch (error) {
      console.error('[剪贴板管理器] 根据ID获取标签组失败:', error)
      return null
    }
  }

  private getTagGroupStats(): any[] {
    try {
      return clipboardDatabase.getTagGroupStats()
    } catch (error) {
      console.error('[剪贴板管理器] 获取标签统计失败:', error)
      return []
    }
  }

  // ==================== 剪贴板操作方法 ====================

  private readAllClipboard(): any {
    try {
      // 检查文本内容
      const text = clipboard.readText()
      const html = clipboard.readHTML()
      const rtf = clipboard.readRTF()
      const bookmark = clipboard.readBookmark()

      // 检查图片是否存在（不读取具体内容）
      const image = clipboard.readImage()
      const hasImage = !image.isEmpty()

      return {
        text: text || undefined,
        html: html || undefined,
        rtf: rtf || undefined,
        bookmark: bookmark?.title ? bookmark : undefined,
        hasImage: hasImage
      }
    } catch (error) {
      console.error('[剪贴板管理器] 读取剪贴板失败:', error)
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
      console.error('[剪贴板管理器] 读取剪贴板内容失败:', error)
    }

    return content
  }

  private restoreClipboard(params: { content: ClipboardContent }): boolean {
    try {
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
          console.warn('[剪贴板管理器] 处理图片数据失败:', e)
        }
      }

      // 一次性写入所有格式
      if (Object.keys(clipboardData).length > 0) {
        clipboard.write(clipboardData)
        console.log('[剪贴板管理器] 已使用 write 方法设置剪贴板，格式:', Object.keys(clipboardData))
      }

      // 单独处理书签（因为 write 方法不支持书签）
      if (content.bookmark) {
        try {
          const bookmark = JSON.parse(content.bookmark)
          clipboard.writeBookmark(bookmark.title, bookmark.url)
          console.log('[剪贴板管理器] 已设置书签:', bookmark.title)
        } catch (e) {
          console.warn('[剪贴板管理器] 设置书签失败:', e)
        }
      }

      return true
    } catch (error) {
      console.error('[剪贴板管理器] 还原剪贴板失败:', error)
      return false
    }
  }

  // ==================== 活动窗口操作方法 ====================

  private getActiveWindow(): any {
    try {
      return ActiveWindow.getActiveWindow()
    } catch (error) {
      console.error('[剪贴板管理器] 获取活动窗口失败:', error)
      return {}
    }
  }

  // ==================== 公共 API 方法 ====================

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    return this.call('initialize', config)
  }

  /**
   * 更新配置
   */
  public async updateConfig(config: Partial<ClipboardManagerConfig>): Promise<void> {
    return this.call('updateConfig', config)
  }

  /**
   * 获取配置
   */
  public async getConfig(): Promise<ClipboardManagerConfig> {
    return this.call('getConfig')
  }

  /**
   * 开始监听剪贴板变化
   */
  public async startListening(): Promise<void> {
    return this.call('startListening')
  }

  /**
   * 停止监听剪贴板变化
   */
  public async stopListening(): Promise<void> {
    return this.call('stopListening')
  }

  /**
   * 检查是否正在监听
   */
  public async isListeningActive(): Promise<boolean> {
    return this.call('isListeningActive')
  }

  /**
   * 获取当前剪贴板内容
   */
  public async getCurrentClipboard(): Promise<ClipboardContent> {
    return this.call('getCurrentClipboard')
  }

  /**
   * 还原剪贴板内容
   */
  public async restoreClipboardItem(itemId: string): Promise<boolean> {
    return this.call('restoreClipboard', itemId)
  }

  /**
   * 同步剪贴板哈希
   */
  public async syncClipboardHash(): Promise<void> {
    return this.call('syncClipboardHash')
  }

  /**
   * 获取历史记录
   */
  public async getHistory(limit?: number): Promise<ClipboardItem[]> {
    return this.call('getHistory', limit)
  }

  /**
   * 删除历史记录项
   */
  public async deleteHistoryItem(itemId: string): Promise<boolean> {
    return this.call('deleteHistoryItem', itemId)
  }

  /**
   * 清空历史记录
   */
  public async clearHistory(): Promise<void> {
    return this.call('clearHistory')
  }

  /**
   * 搜索历史记录
   */
  public async searchHistory(options: any): Promise<ClipboardItem[]> {
    return this.call('searchHistory', options)
  }

  /**
   * 同步删除的记录
   */
  public async syncDeletedRecords(deletedIds: string[]): Promise<void> {
    return this.call('syncDeletedRecords', deletedIds)
  }

  // ==================== 标签管理方法 ====================

  /**
   * 创建标签组
   */
  public async createTagGroup(name: string, color: string, description?: string): Promise<string | null> {
    return this.call('createTagGroup', { name, color, description })
  }

  /**
   * 获取所有标签组
   */
  public async getAllTagGroups(): Promise<any[]> {
    return this.call('getAllTagGroups')
  }

  /**
   * 更新标签组
   */
  public async updateTagGroup(tagId: string, updates: any): Promise<boolean> {
    return this.call('updateTagGroup', { tagId, updates })
  }

  /**
   * 删除标签组
   */
  public async deleteTagGroup(tagId: string): Promise<boolean> {
    return this.call('deleteTagGroup', tagId)
  }

  /**
   * 绑定标签到记录
   */
  public async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    return this.call('bindTagToRecord', { contentId, tagId })
  }

  /**
   * 解绑标签
   */
  public async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    return this.call('unbindTagFromRecord', { contentId, tagId })
  }

  /**
   * 按标签获取记录
   */
  public async getRecordsByTags(tagIds: string[]): Promise<ClipboardItem[]> {
    return this.call('getRecordsByTags', tagIds)
  }

  /**
   * 按解析类型获取记录
   */
  public async getRecordsByParseType(parseType: string): Promise<ClipboardItem[]> {
    return this.call('getRecordsByParseType', parseType)
  }

  /**
   * 获取标签统计
   */
  public async getTagStats(): Promise<any[]> {
    return this.call('getTagStats')
  }
}
