import { parentPort } from 'node:worker_threads'
import { HashUtils } from '../utils/common'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type {
  ClipboardContent,
  ClipboardItem,
  ClipboardManagerConfig,
  TagGroup,
  TagStats,
  AppInfo
} from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板管理器 Worker
 * 在独立线程中运行，负责剪贴板历史记录管理和数据库操作
 */
class ClipboardManagerWorker {
  private config: ClipboardManagerConfig
  private isListening: boolean = false
  private checkTimer: NodeJS.Timeout | null = null
  private lastClipboardHash: string = ''
  private lastQuickHash: string = ''
  private historyItems: ClipboardItem[] = []

  constructor() {
    // 默认配置
    this.config = {
      maxHistorySize: 100,
      checkInterval: 200,
      enableAutoSave: false
    }


  }

  /**
   * 初始化剪贴板管理器
   */
  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    if (config) {
      this.config = { ...this.config, ...config }
    }

    try {
      // 通过主线程初始化数据库
      await this.callMainThread('database-initialize')

      // 从数据库加载历史记录
      await this.loadHistoryFromDatabase()

      // 初始化时获取当前剪贴板内容哈希
      this.lastClipboardHash = await this.generateClipboardHash()

      // 初始化快速哈希
      await this.hasQuickClipboardChange()
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 初始化失败:', error)
      // 即使数据库初始化失败，也要继续初始化剪贴板管理器
      this.lastClipboardHash = await this.generateClipboardHash()
      await this.hasQuickClipboardChange()
    }
  }

  /**
   * 开始监听剪贴板变化
   */
  public startListening(): void {
    if (this.isListening) {
      return
    }

    this.isListening = true
    this.checkTimer = setInterval(() => {
      this.checkClipboardChange()
    }, this.config.checkInterval)
  }

  /**
   * 停止监听剪贴板变化
   */
  public stopListening(): void {
    if (!this.isListening) {
      return
    }

    this.isListening = false
    if (this.checkTimer) {
      clearInterval(this.checkTimer)
      this.checkTimer = null
    }
  }

  /**
   * 检查剪贴板内容变化
   */
  private async checkClipboardChange(): Promise<void> {
    try {
      // 首先进行快速检查：只检查是否有内容和基本格式
      const hasQuickChange = await this.hasQuickClipboardChange()

      if (!hasQuickChange) {
        return
      }

      // 如果快速检查发现变化，再进行完整的哈希计算
      const currentHash = await this.generateClipboardHash()
      if (currentHash && currentHash !== this.lastClipboardHash) {
        this.lastClipboardHash = currentHash

        if (this.config.enableAutoSave) {
          const clipboardContent = await this.getCurrentClipboardContent()
          await this.saveClipboardItem(clipboardContent)
        }

        this.sendEvent('clipboard-changed', { timestamp: Date.now() })
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 检查剪贴板变化失败:', error)
    }
  }

  /**
   * 快速检查剪贴板是否有变化
   */
  private async hasQuickClipboardChange(): Promise<boolean> {
    try {
      // 通过主线程获取剪贴板内容
      const clipboardData = await this.callMainThread('clipboard-readAll')
      
      // 生成快速哈希（不包含图片具体内容）
      const quickContent = {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        bookmark: clipboardData.bookmark ? JSON.stringify(clipboardData.bookmark) : undefined,
        hasImage: clipboardData.hasImage
      }

      const quickHash = this.generateQuickHash(quickContent)

      // 如果没有上次的快速哈希，说明是第一次检查
      if (!this.lastQuickHash) {
        this.lastQuickHash = quickHash
        return true
      }

      // 比较快速哈希
      const hasChange = quickHash !== this.lastQuickHash
      if (hasChange) {
        this.lastQuickHash = quickHash
      }

      return hasChange
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 快速检查失败:', error)
      return true // 出错时假设有变化，进行完整检查
    }
  }

  /**
   * 生成快速哈希
   */
  private generateQuickHash(content: any): string {
    return HashUtils.generateQuickHash(content)
  }

  /**
   * 获取当前剪贴板所有格式的内容
   */
  private async getCurrentClipboardContent(): Promise<ClipboardContent> {
    try {
      // 通过主线程获取剪贴板内容
      return await this.callMainThread('clipboard-readContent')
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 读取剪贴板内容失败:', error)
      return {}
    }
  }

  /**
   * 生成剪贴板内容的哈希值用于比较
   */
  private async generateClipboardHash(): Promise<string> {
    try {
      const content = await this.getCurrentClipboardContent()
      return ClipboardDataConverter.generateContentHash(content)
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 生成剪贴板哈希失败:', error)
      return ''
    }
  }

  /**
   * 保存剪贴板项到历史记录
   */
  private async saveClipboardItem(content: ClipboardContent): Promise<void> {
    if (!content || ClipboardDataConverter.isEmptyContent(content)) {
      return
    }

    try {
      const now = Date.now()
      const contentHash = ClipboardDataConverter.generateContentHash(content)

      // 检查是否已存在相同内容
      const existingIndex = this.historyItems.findIndex(item =>
        ClipboardDataConverter.generateContentHash(item.content) === contentHash
      )

      if (existingIndex !== -1) {
        // 更新现有项的最后使用时间并移到最前面
        const existingItem = this.historyItems[existingIndex]
        existingItem.lastUsed = now
        this.historyItems.splice(existingIndex, 1)
        this.historyItems.unshift(existingItem)

        // 同步更新数据库中的最后使用时间
        await this.syncToDatabase(existingItem, true)
      } else {
        // 获取当前激活窗口的应用信息
        const sourceAppInfo = await this.getCurrentAppInfo()

        // 创建新的剪贴板项
        const availableFormats = ClipboardDataConverter.getAvailableFormats(content)
        const newItem: ClipboardItem = {
          id: ClipboardDataConverter.generateId(),
          content,
          primaryType: ClipboardDataConverter.detectPrimaryType(content, availableFormats),
          availableFormats: availableFormats,
          timestamp: now,
          lastUsed: now,
          parseType: ClipboardDataConverter.detectParseType(content, availableFormats),
          tags: [],
          sourceAppInfo: sourceAppInfo
        }

        // 添加到历史记录开头
        this.historyItems.unshift(newItem)

        // 同步保存到数据库
        await this.syncToDatabase(newItem, false)

        // 如果超过最大数量，删除最久未使用的项
        if (this.historyItems.length > this.config.maxHistorySize) {
          const removedItems = this.historyItems.splice(this.config.maxHistorySize)
          // 从数据库中删除被移除的项
          for (const item of removedItems) {
            await this.removeFromDatabase(item.id)
          }
        }
      }

      // 通知前端剪贴板历史已更新
      this.sendEvent('history-updated', { timestamp: Date.now() })
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 保存剪贴板项失败:', error)
    }
  }

  /**
   * 从数据库加载历史记录
   */
  private async loadHistoryFromDatabase(): Promise<void> {
    try {
      const recordsWithTags = await this.callMainThread('database-queryRecordsWithTags', {
        limit: this.config.maxHistorySize,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      this.historyItems = ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 从数据库加载历史记录失败:', error)
      this.historyItems = []
    }
  }

  /**
   * 同步剪贴板项到数据库
   */
  private async syncToDatabase(item: ClipboardItem, isUpdate: boolean): Promise<void> {
    try {
      if (isUpdate) {
        // 更新最后使用时间
        await this.callMainThread('database-updateLastUsed', {
          contentId: item.id,
          lastUsed: item.lastUsed
        })
      } else {
        // 如果有应用信息，先保存应用信息到数据库
        let sourceAppId: string | undefined = undefined
        if (item.sourceAppInfo) {
          sourceAppId = await this.callMainThread('database-insertOrUpdateApp', {
            application: item.sourceAppInfo.application,
            path: item.sourceAppInfo.path,
            icon: item.sourceAppInfo.icon
          })
        }

        // 插入新记录
        await this.callMainThread('database-insertRecord', {
          contentId: item.id,
          content: item.content,
          primaryType: item.primaryType,
          availableFormats: item.availableFormats,
          timestamp: item.timestamp,
          lastUsed: item.lastUsed,
          parseType: item.parseType,
          sourceAppId
        })
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 同步到数据库失败:', error)
    }
  }

  /**
   * 从数据库中删除记录
   */
  private async removeFromDatabase(contentId: string): Promise<void> {
    try {
      await this.callMainThread('database-deleteRecord', { contentId })
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 从数据库删除记录失败:', error)
    }
  }

  /**
   * 同步删除内存中的记录（由清理器调用）
   */
  public syncDeletedRecords(deletedIds: string[]): void {
    try {
      const originalLength = this.historyItems.length

      // 从内存中删除指定ID的记录
      this.historyItems = this.historyItems.filter(item => !deletedIds.includes(item.id))

      const deletedCount = originalLength - this.historyItems.length

      if (deletedCount > 0) {
        this.sendEvent('history-updated', { timestamp: Date.now(), source: 'cleanup' })
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 同步删除内存记录失败:', error)
    }
  }

  /**
   * 获取历史记录
   */
  public getHistory(limit?: number): ClipboardItem[] {
    if (limit && limit > 0) {
      return [...this.historyItems.slice(0, limit)]
    }
    return [...this.historyItems]
  }

  /**
   * 还原指定的剪贴板内容到当前剪贴板
   */
  public async restoreClipboard(itemId: string): Promise<boolean> {
    try {
      const item = this.historyItems.find(h => h.id === itemId)

      if (!item) {
        log.error('[剪贴板管理器 Worker] 未找到指定的剪贴板项:', itemId)
        return false
      }

      // 通过主线程还原剪贴板内容
      const success = await this.callMainThread('clipboard-restore', {
        content: item.content
      })

      if (success) {
        // 更新最后使用时间并重新排序
        item.lastUsed = Date.now()
        const itemIndex = this.historyItems.findIndex(h => h.id === itemId)
        if (itemIndex > 0) {
          this.historyItems.splice(itemIndex, 1)
          this.historyItems.unshift(item)
        }

        // 同步更新数据库中的最后使用时间
        await this.syncToDatabase(item, true)

        return true
      } else {
        log.error('[剪贴板管理器 Worker] 还原失败:', itemId)
        return false
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 还原剪贴板失败:', error)
      return false
    }
  }

  /**
   * 还原剪贴板内容为纯文本
   * @param itemId 剪贴板项目ID
   * @returns Promise<boolean> 是否成功还原
   */
  private async restoreClipboardAsText(itemId: string): Promise<boolean> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        log.error('[剪贴板管理器 Worker] 还原为纯文本失败: 无效的项目ID:', itemId)
        return false
      }

      const item = this.historyItems.find(h => h.id === itemId)

      if (!item) {
        log.error('[剪贴板管理器 Worker] 未找到指定的剪贴板项:', itemId)
        return false
      }

      // 检查是否有文本内容
      if (!item.content.text || !item.content.text.trim()) {
        return false
      }

      // 通过主线程还原剪贴板内容为纯文本
      const success = await this.callMainThread('clipboard-restore-as-text', {
        content: item.content
      })

      if (success) {
        // 更新最后使用时间并重新排序
        item.lastUsed = Date.now()
        const itemIndex = this.historyItems.findIndex(h => h.id === itemId)
        if (itemIndex > 0) {
          this.historyItems.splice(itemIndex, 1)
          this.historyItems.unshift(item)
        }

        // 同步更新数据库中的最后使用时间
        await this.syncToDatabase(item, true)

        return true
      } else {
        log.error('[剪贴板管理器 Worker] 主线程还原为纯文本失败:', itemId)
        return false
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 还原剪贴板为纯文本时发生异常:', error, '项目ID:', itemId)
      return false
    }
  }

  /**
   * 删除指定的历史记录项
   */
  public async deleteHistoryItem(itemId: string): Promise<boolean> {
    try {
      const itemIndex = this.historyItems.findIndex(h => h.id === itemId)

      if (itemIndex === -1) {
        log.error('[剪贴板管理器 Worker] 未找到指定的剪贴板项:', itemId)
        return false
      }

      // 从内存中删除
      this.historyItems.splice(itemIndex, 1)

      // 从数据库中删除
      await this.removeFromDatabase(itemId)

      this.sendEvent('history-updated', { timestamp: Date.now() })
      return true
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 删除剪贴板项失败:', error)
      return false
    }
  }

  /**
   * 清空所有历史记录
   */
  public async clearHistory(): Promise<void> {
    try {
      // 清空内存中的历史记录
      this.historyItems = []

      // 清空数据库中的历史记录
      await this.callMainThread('database-clearAllRecords')

      this.sendEvent('history-updated', { timestamp: Date.now() })
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 清空历史记录失败:', error)
    }
  }

  /**
   * 获取配置
   */
  public getConfig(): ClipboardManagerConfig {
    return { ...this.config }
  }

  /**
   * 更新配置
   */
  public updateConfig(newConfig: Partial<ClipboardManagerConfig>): void {
    this.config = { ...this.config, ...newConfig }
  }

  /**
   * 获取监听状态
   */
  public isListeningActive(): boolean {
    return this.isListening
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   */
  public async syncClipboardHash(): Promise<void> {
    try {
      this.lastClipboardHash = await this.generateClipboardHash()
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 同步剪贴板哈希值失败:', error)
    }
  }

  /**
   * 获取当前剪贴板内容（不触发监控）
   */
  public async getCurrentClipboard(): Promise<ClipboardContent> {
    return this.getCurrentClipboardContent()
  }

  /**
   * 获取当前激活窗口的应用信息
   */
  private async getCurrentAppInfo(): Promise<AppInfo | undefined> {
    try {
      const activeWindow = await this.callMainThread('activeWindow-getActiveWindow')

      if (!activeWindow.application || !activeWindow.path) {
        return undefined
      }

      return {
        id: ClipboardDataConverter.generateAppId(),
        application: activeWindow.application,
        path: activeWindow.path,
        icon: activeWindow.icon,
        createdAt: Date.now(),
        updatedAt: Date.now()
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 获取当前应用信息失败:', error)
      return undefined
    }
  }

  // ==================== 标签管理方法 ====================

  /**
   * 创建新标签组
   */
  public async createTagGroup(name: string, color: string, description?: string): Promise<string | null> {
    try {
      const tagId = ClipboardDataConverter.generateTagId()
      const now = Date.now()

      const tag: TagGroup = {
        id: tagId,
        name,
        color,
        description,
        createdAt: now,
        updatedAt: now
      }

      const success = await this.callMainThread('database-createTagGroup', { tag })
      if (success) {
        return tagId
      } else {
        log.error('[剪贴板管理器 Worker] 创建标签组失败')
        return null
      }
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 获取所有标签组
   */
  public async getAllTagGroups(): Promise<TagGroup[]> {
    try {
      const records = await this.callMainThread('database-getAllTagGroups')
      return records.map((record: any) => ClipboardDataConverter.tagRecordToTag(record))
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 获取标签组列表失败:', error)
      return []
    }
  }

  /**
   * 更新标签组
   */
  public async updateTagGroup(tagId: string, updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>>): Promise<boolean> {
    try {
      const updateData = {
        ...updates,
        updatedAt: Date.now()
      }

      const success = await this.callMainThread('database-updateTagGroup', { tagId, updateData })
      return success
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 更新标签组失败:', error)
      return false
    }
  }

  /**
   * 删除标签组
   */
  public async deleteTagGroup(tagId: string): Promise<boolean> {
    try {
      const success = await this.callMainThread('database-deleteTagGroup', { tagId })
      if (success) {
        // 重新加载历史记录以更新标签信息
        await this.loadHistoryFromDatabase()
      }
      return success
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 删除标签组失败:', error)
      return false
    }
  }

  /**
   * 为历史记录绑定标签
   */
  public async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      const success = await this.callMainThread('database-bindTagToRecord', { contentId, tagId })
      if (success) {
        // 更新内存中的历史记录
        const item = this.historyItems.find(item => item.id === contentId)
        if (item) {
          const tagRecord = await this.callMainThread('database-getTagGroupById', { tagId })
          if (tagRecord) {
            const tag = ClipboardDataConverter.tagRecordToTag(tagRecord)
            if (!item.tags) item.tags = []
            if (!item.tags.find(t => t.id === tagId)) {
              item.tags.push(tag)
            }
          }
        }
      }
      return success
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 绑定标签失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的标签绑定
   */
  public async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      const success = await this.callMainThread('database-unbindTagFromRecord', { contentId, tagId })
      if (success) {
        // 更新内存中的历史记录
        const item = this.historyItems.find(item => item.id === contentId)
        if (item && item.tags) {
          item.tags = item.tags.filter(tag => tag.id !== tagId)
        }
      }
      return success
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 解除标签绑定失败:', error)
      return false
    }
  }

  /**
   * 按标签筛选历史记录
   */
  public async getRecordsByTags(tagIds: string[]): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await this.callMainThread('database-queryRecordsWithTags', {
        tagIds,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 按标签筛选记录失败:', error)
      return []
    }
  }

  /**
   * 按解析类型筛选历史记录
   */
  public async getRecordsByParseType(parseType: string): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await this.callMainThread('database-queryRecordsWithTags', {
        parseType,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 按解析类型筛选记录失败:', error)
      return []
    }
  }

  /**
   * 获取标签统计信息
   */
  public async getTagStats(): Promise<TagStats[]> {
    try {
      return await this.callMainThread('database-getTagGroupStats')
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 获取标签统计信息失败:', error)
      return []
    }
  }

  /**
   * 搜索历史记录（支持标签和内容筛选）
   */
  public async searchHistory(options: {
    limit?: number
    offset?: number
    tagIds?: string[]
    parseType?: string
    searchText?: string
    orderBy?: 'timestamp' | 'last_used'
    orderDirection?: 'ASC' | 'DESC'
  } = {}): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await this.callMainThread('database-queryRecordsWithTags', {
        limit: options.limit,
        offset: options.offset,
        tagIds: options.tagIds,
        parseType: options.parseType,
        searchText: options.searchText,
        orderBy: options.orderBy || 'last_used',
        orderDirection: options.orderDirection || 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板管理器 Worker] 搜索历史记录失败:', error)
      return []
    }
  }

  /**
   * 调用主线程方法
   */
  private async callMainThread(method: string, params?: any): Promise<any> {
    return new Promise((resolve, reject) => {
      const requestId = `worker-${Date.now()}-${Math.random()}`

      const request: WorkerRequest = {
        id: requestId,
        type: 'request',
        method,
        params,
        timestamp: Date.now()
      }

      // 设置超时
      const timeout = setTimeout(() => {
        reject(new Error(`调用主线程方法超时: ${method}`))
      }, 30000)

      // 监听响应
      const messageHandler = (message: WorkerMessage) => {
        if (message.type === 'response' && (message as WorkerResponse).requestId === requestId) {
          clearTimeout(timeout)
          parentPort?.off('message', messageHandler)

          const response = message as WorkerResponse
          if (response.success) {
            resolve(response.data)
          } else {
            reject(new Error(response.error || '主线程调用失败'))
          }
        }
      }

      parentPort?.on('message', messageHandler)
      parentPort?.postMessage(request)
    })
  }

  /**
   * 发送事件到主线程
   */
  private sendEvent(event: string, data?: any): void {
    const eventMessage: WorkerEvent = {
      id: `event-${Date.now()}-${Math.random()}`,
      type: 'event',
      event,
      data,
      timestamp: Date.now()
    }
    parentPort?.postMessage(eventMessage)
  }

  /**
   * 处理来自主线程的消息
   */
  public handleMessage(message: WorkerMessage): void {
    if (message.type === 'request') {
      this.handleRequest(message as WorkerRequest)
    }
  }

  /**
   * 处理请求
   */
  private async handleRequest(request: WorkerRequest): Promise<void> {
    try {
      let result: any

      switch (request.method) {
        case 'initialize':
          result = await this.initialize(request.params)
          break
        case 'updateConfig':
          result = this.updateConfig(request.params)
          break
        case 'getConfig':
          result = this.getConfig()
          break
        case 'startListening':
          result = this.startListening()
          break
        case 'stopListening':
          result = this.stopListening()
          break
        case 'isListeningActive':
          result = this.isListeningActive()
          break
        case 'getCurrentClipboard':
          result = await this.getCurrentClipboard()
          break
        case 'restoreClipboard':
          result = await this.restoreClipboard(request.params)
          break
        case 'restoreClipboardAsText':
          result = await this.restoreClipboardAsText(request.params)
          break
        case 'syncClipboardHash':
          result = await this.syncClipboardHash()
          break
        case 'getHistory':
          result = this.getHistory(request.params)
          break
        case 'deleteHistoryItem':
          result = await this.deleteHistoryItem(request.params)
          break
        case 'clearHistory':
          result = await this.clearHistory()
          break
        case 'searchHistory':
          result = await this.searchHistory(request.params)
          break
        case 'createTagGroup':
          result = await this.createTagGroup(request.params.name, request.params.color, request.params.description)
          break
        case 'getAllTagGroups':
          result = await this.getAllTagGroups()
          break
        case 'updateTagGroup':
          result = await this.updateTagGroup(request.params.tagId, request.params.updates)
          break
        case 'deleteTagGroup':
          result = await this.deleteTagGroup(request.params)
          break
        case 'bindTagToRecord':
          result = await this.bindTagToRecord(request.params.contentId, request.params.tagId)
          break
        case 'unbindTagFromRecord':
          result = await this.unbindTagFromRecord(request.params.contentId, request.params.tagId)
          break
        case 'getRecordsByTags':
          result = await this.getRecordsByTags(request.params)
          break
        case 'getRecordsByParseType':
          result = await this.getRecordsByParseType(request.params)
          break
        case 'getTagStats':
          result = await this.getTagStats()
          break
        case 'syncDeletedRecords':
          result = this.syncDeletedRecords(request.params)
          break
        default:
          throw new Error(`未知方法: ${request.method}`)
      }

      // 发送成功响应
      const response: WorkerResponse = {
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: true,
        data: result,
        timestamp: Date.now()
      }
      parentPort?.postMessage(response)
    } catch (error) {
      // 发送错误响应
      const response: WorkerResponse = {
        id: `response-${Date.now()}`,
        type: 'response',
        requestId: request.id,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        timestamp: Date.now()
      }
      parentPort?.postMessage(response)
    }
  }
}

// 创建 Worker 实例
const worker = new ClipboardManagerWorker()

// 监听主线程消息
parentPort?.on('message', (message: WorkerMessage) => {
  worker.handleMessage(message)
})

// Worker 准备就绪，发送 ready 事件
// 发送 worker-ready 事件，告知主线程 Worker 已启动
const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: {},
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
