import { parentPort } from 'node:worker_threads'
import { HashUtils } from '../utils/common'
import { BaseWorker } from './base-worker'
import { ClipboardManager as TeddyClipboardManager, ClipboardListener, ClipboardData } from '@teddyzhu/clipboard'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type {
  ClipboardContent,
  ClipboardManagerConfig,
  AppInfo
} from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import ActiveWindow from '@paymoapp/active-window'

class ClipboardManagerWorker extends BaseWorker {
  private config: ClipboardManagerConfig
  private clipboard: TeddyClipboardManager
  private clipboardListener: ClipboardListener
  private isListening: boolean = false
  private checkTimer: NodeJS.Timeout | null = null
  private lastClipboardHash: string = ''
  private lastQuickHash: string = ''
  private lastItemId: string = ''
  private lastItemHash: string = ''
  private activeWindowInitialized: boolean = false

  // 历史记录缓存：hash -> { id, hash }
  private historyCache: Map<string, { id: string, hash: string }> = new Map()
  // 反向索引：id -> hash，用于根据 ID 快速查找和删除
  private idToHashIndex: Map<string, string> = new Map()
  private maxCacheSize: number = 10000 // 最大缓存条目数

  constructor() {
    super()
    this.config = {
      maxHistorySize: 100,
      checkInterval: 200,
      enableAutoSave: false
    }

    // 初始化剪贴板管理器
    this.clipboard = new TeddyClipboardManager()

    // 初始化剪贴板监听器
    this.clipboardListener = new ClipboardListener()

    // 初始化 ActiveWindow
    this.initializeActiveWindow()
  }

  protected getWorkerName(): string {
    return '剪贴板变化检测 Worker'
  }

  /**
   * 直接读取剪贴板内容（使用 @teddyzhu/clipboard）
   * 现在使用新的 getImageData API 获取完整的图片信息
   */
  private readClipboardDirect(): any {
    try {
      const text = this.clipboard.getText()
      const html = this.clipboard.getHtml()
      const rtf = this.clipboard.getRichText()

      // 使用新的 getImageData API 获取完整图片信息
      let imageData: any = null
      let hasImage = false

      try {
        imageData = this.clipboard.getImageData()
        hasImage = !!(imageData && imageData.base64Data)
      } catch (e) {
        // 如果没有图片或获取失败，使用旧的 API
        const imageBase64 = this.clipboard.getImageBase64()
        hasImage = !!imageBase64
        if (hasImage) {
          imageData = {
            width: 0,
            height: 0,
            size: imageBase64.length,
            base64Data: imageBase64
          }
        }
      }

      const result: any = {
        text: text || undefined,
        html: html || undefined,
        rtf: rtf || undefined,
        hasImage: hasImage
      }

      // @teddyzhu/clipboard 不支持书签
      result.bookmark = undefined

      // 如果有图片，添加图片的基本信息用于快速变化检测
      if (hasImage && imageData) {
        result.imageInfo = {
          width: imageData.width || 0,
          height: imageData.height || 0,
          quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
        }
        result.image = imageData.base64Data
      }

      return result
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 直接读取剪贴板失败:', error)
      return {}
    }
  }

  /**
   * 初始化 ActiveWindow
   */
  private initializeActiveWindow(): void {
    try {
      ActiveWindow.initialize()
      // 在 macOS 上请求屏幕录制权限
      if (process.platform === 'darwin') {
        const hasPermission = ActiveWindow.requestPermissions()
        if (!hasPermission) {
          console.warn('[剪贴板管理器 Worker] macOS 需要屏幕录制权限才能获取窗口标题')
        }
      }
      this.activeWindowInitialized = true
      console.log('[剪贴板管理器 Worker] ActiveWindow 初始化成功')
    } catch (error) {
      console.error('[剪贴板管理器 Worker] ActiveWindow 初始化失败:', error)
      this.activeWindowInitialized = false
    }
  }

  public async initialize(config?: Partial<ClipboardManagerConfig>): Promise<void> {
    if (config) {
      this.config = { ...this.config, ...config }
    }

    try {
      // 初始化历史记录缓存
      await this.initializeHistoryCache()

      // 数据库已在主进程中初始化，直接加载最后一项信息
      await this.loadLastItemInfo()
      // 直接获取剪贴板数据进行初始化
      const clipboardData = this.readClipboardDirect()
      this.lastClipboardHash = await this.generateClipboardHashFromData(clipboardData)
      this.checkQuickClipboardChange(clipboardData)
    } catch (error) {
      console.log('初始化失败')
    }
  }

  public startListening(): void {
    if (this.isListening) return

    this.isListening = true

    // 使用 ClipboardListener 监听剪贴板变化
    this.clipboardListener.watch((data: ClipboardData) => {
      this.handleClipboardChange(data)
    })
  }

  public stopListening(): void {
    if (!this.isListening) return

    this.isListening = false

    // 停止 ClipboardListener
    this.clipboardListener.stop()

    // 清理定时器（如果有的话）
    if (this.checkTimer) {
      clearInterval(this.checkTimer)
      this.checkTimer = null
    }
  }

  /**
   * 处理剪贴板变化（使用 ClipboardListener 回调）
   */
  private async handleClipboardChange(data: ClipboardData): Promise<void> {
    try {
      if (this.isInErrorBackoff()) return

      console.log('clipboard changed via listener', new Date().toDateString())

      // 将 ClipboardData 转换为内部格式
      const clipboardData = this.convertClipboardData(data)

      // 检查快速变化（基于文本、HTML、RTF、书签和图片存在性）
      const hasQuickChange = this.checkQuickClipboardChange(clipboardData)
      if (!hasQuickChange) {
        this.onOperationSuccess()
        return
      }
      console.log('quick failed , check full hash', new Date().toDateString())

      // 生成完整内容哈希
      const currentHash = await this.generateClipboardHashFromData(clipboardData)
      console.log('full hash', currentHash)
      if (currentHash && currentHash !== this.lastClipboardHash) {
        this.lastClipboardHash = currentHash

        if (this.config.enableAutoSave) {
          // 从已获取的数据构建完整内容
          console.log('build content from data', new Date().toDateString())
          const clipboardContent = await this.buildClipboardContentFromData(clipboardData)
          console.log('save clipboard item', new Date().toDateString())
          await this.saveClipboardItem(clipboardContent)
          console.log('save clipboard item done', new Date().toDateString())
        }

        this.sendEvent('clipboard-changed', { timestamp: Date.now() })
      }

      this.onOperationSuccess()
    } catch (error) {
      this.handleCheckError(error, '剪贴板检查')
    }
  }

  /**
   * 将 ClipboardData 转换为内部使用的格式
   */
  private convertClipboardData(data: ClipboardData): any {
    const result: any = {
      text: data.text || undefined,
      html: data.html || undefined,
      rtf: data.rtf || undefined,
      hasImage: !!data.image
    }

    // 处理书签 - @teddyzhu/clipboard 不支持书签
    result.bookmark = undefined

    // 处理图片数据
    if (data.image) {
      const imageData = data.image as any // 类型断言，因为 ImageData 接口包含这些属性
      result.imageInfo = {
        width: imageData.width || 0,
        height: imageData.height || 0,
        quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
      }
      result.image = imageData.base64Data || imageData
    }

    return result
  }

  /**
   * 检查快速剪贴板变化 - 基于已获取的数据
   * @param clipboardData 已获取的剪贴板数据
   * @returns 是否有变化
   */
  private checkQuickClipboardChange(clipboardData: any): boolean {
    try {
      const quickContent = {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        bookmark: clipboardData.bookmark ? JSON.stringify(clipboardData.bookmark) : undefined,
        hasImage: clipboardData.hasImage,
        // 包含图片的基本信息用于区分不同的图片
        imageInfo: clipboardData.imageInfo || undefined
      }
      if(quickContent.hasImage) {
        console.log('has image', quickContent.imageInfo)
      }
      const quickHash = this.generateQuickHash(quickContent)
      if(quickContent.hasImage) {
        console.log('quick hash changed', quickHash, this.lastQuickHash)
      }
      if (!this.lastQuickHash) {
        this.lastQuickHash = quickHash
        return true
      }

      const hasChange = quickHash !== this.lastQuickHash
      if (hasChange) {
        this.lastQuickHash = quickHash
      }

      return hasChange
    } catch (error) {
      return true
    }
  }

  private generateQuickHash(content: any): string {
    return HashUtils.generateQuickHash(content)
  }

  /**
   * 基于已获取的剪贴板数据生成内容哈希
   * @param clipboardData 已获取的剪贴板数据
   * @returns 内容哈希值
   */
  private async generateClipboardHashFromData(clipboardData: any): Promise<string> {
    try {
      const content = await this.buildClipboardContentFromData(clipboardData)
      return ClipboardDataConverter.generateContentHash(content)
    } catch (error) {
      return ''
    }
  }

  /**
   * 基于已获取的剪贴板数据构建完整的剪贴板内容
   * 优化策略：只有在需要图片数据且没有图片数据时才额外获取
   * @param clipboardData 已获取的剪贴板数据
   * @returns 完整的剪贴板内容
   */
  private async buildClipboardContentFromData(clipboardData: any): Promise<ClipboardContent> {
    try {
      // 如果已经有图片数据，直接使用
      if (clipboardData.hasImage && clipboardData.image) {
        return {
          text: clipboardData.text || undefined,
          html: clipboardData.html || undefined,
          rtf: clipboardData.rtf || undefined,
          bookmark: clipboardData.bookmark ? JSON.stringify(clipboardData.bookmark) : undefined,
          image: clipboardData.image
        }
      }

      // 没有图片，直接构建内容
      return {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        bookmark: clipboardData.bookmark ? JSON.stringify(clipboardData.bookmark) : undefined
      }
    } catch (error) {
      return {}
    }
  }

  private async saveClipboardItem(content: ClipboardContent): Promise<void> {
    if (!content || ClipboardDataConverter.isEmptyContent(content)) return

    try {
      const now = Date.now()
      const contentHash = ClipboardDataConverter.generateContentHash(content)

      if (this.lastItemHash === contentHash) {
        await this.callMainThread('database-updateLastUsed', {
          contentId: this.lastItemId,
          lastUsed: now
        })
        this.sendEvent('history-updated', { timestamp: Date.now() })
        return
      }

      // 优化：使用缓存查找记录而不是数据库查询
      const cachedRecord = this.getRecordFromCache(contentHash)

      if (cachedRecord) {
        await this.callMainThread('database-updateLastUsed', {
          contentId: cachedRecord.id,
          lastUsed: now
        })
        this.lastItemId = cachedRecord.id
        this.lastItemHash = contentHash
      } else {
        const sourceAppInfo = await this.getCurrentAppInfo()
        const availableFormats = ClipboardDataConverter.getAvailableFormats(content)
        const newItemId = ClipboardDataConverter.generateId()

        let sourceAppId: string | undefined = undefined
        if (sourceAppInfo) {
          sourceAppId = await this.callMainThread('database-insertOrUpdateApp', {
            application: sourceAppInfo.application,
            path: sourceAppInfo.path,
            icon: sourceAppInfo.icon
          })
        }

        await this.callMainThread('database-insertRecord', {
          contentId: newItemId,
          content,
          primaryType: ClipboardDataConverter.detectPrimaryType(content, availableFormats),
          availableFormats: availableFormats,
          timestamp: now,
          lastUsed: now,
          parseType: ClipboardDataConverter.detectParseType(content, availableFormats),
          sourceAppId,
          hash: contentHash
        })

        // 添加到缓存
        this.addToCache(newItemId, contentHash)

        this.lastItemId = newItemId
        this.lastItemHash = contentHash

        this.sendEvent('cleanup-check-required', {
          reason: 'new-record-added',
          maxHistorySize: this.config.maxHistorySize
        })
      }

      this.sendEvent('history-updated', { timestamp: Date.now() })
    } catch (error) {
      // 静默处理错误，避免过多日志
    }
  }

  /**
   * 初始化历史记录缓存
   * 从数据库加载所有记录的 id 和 hash 到内存缓存
   * 优化：使用轻量级查询，只获取 content_id 和 hash，避免加载完整记录数据
   */
  private async initializeHistoryCache(): Promise<void> {
    try {
      // 使用轻量级查询，只获取 content_id 和 hash
      const records = await this.callMainThread('database-queryRecordIdsAndHashes', {
        limit: this.maxCacheSize,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      this.historyCache.clear()
      this.idToHashIndex.clear()

      for (const record of records) {
        this.historyCache.set(record.hash, {
          id: record.content_id,
          hash: record.hash
        })
        // 维护反向索引
        this.idToHashIndex.set(record.content_id, record.hash)
      }
    } catch (error) {
      // 静默处理错误，缓存初始化失败不影响主要功能
    }
  }

  /**
   * 从缓存中查找记录
   * 替代 database-getRecordByHash 查询
   */
  private getRecordFromCache(hash: string): { id: string, hash: string } | null {
    return this.historyCache.get(hash) || null
  }

  /**
   * 向缓存中添加记录
   */
  private addToCache(id: string, hash: string): void {
    // 如果缓存已满，删除最旧的条目（简单的 LRU 策略）
    if (this.historyCache.size >= this.maxCacheSize) {
      const firstKey = this.historyCache.keys().next().value
      if (firstKey) {
        const oldRecord = this.historyCache.get(firstKey)
        if (oldRecord) {
          this.idToHashIndex.delete(oldRecord.id)
        }
        this.historyCache.delete(firstKey)
      }
    }

    this.historyCache.set(hash, { id, hash })
    this.idToHashIndex.set(id, hash)
  }

  /**
   * 从缓存中移除记录（通过 hash）
   */
  private removeFromCache(hash: string): void {
    const record = this.historyCache.get(hash)
    if (record) {
      this.idToHashIndex.delete(record.id)
    }
    this.historyCache.delete(hash)
  }

  /**
   * 从缓存中移除记录（通过 ID）
   */
  private removeFromCacheById(id: string): void {
    const hash = this.idToHashIndex.get(id)
    if (hash) {
      this.historyCache.delete(hash)
      this.idToHashIndex.delete(id)
    }
  }

  /**
   * 批量从缓存中移除记录（通过 hash）
   */
  private removeMultipleFromCache(hashes: string[]): void {
    for (const hash of hashes) {
      this.removeFromCache(hash)
    }
  }

  /**
   * 批量从缓存中移除记录（通过 ID）
   */
  private removeMultipleFromCacheByIds(ids: string[]): void {
    for (const id of ids) {
      this.removeFromCacheById(id)
    }
  }

  /**
   * 清空缓存
   */
  private clearCache(): void {
    this.historyCache.clear()
    this.idToHashIndex.clear()
  }

  private async loadLastItemInfo(): Promise<void> {
    try {
      // 优化：使用轻量级查询，直接获取最后记录的 content_id 和 hash
      const lastRecordInfo = await this.callMainThread('database-getLastRecordInfo')

      if (lastRecordInfo) {
        this.lastItemId = lastRecordInfo.content_id
        this.lastItemHash = lastRecordInfo.hash
      } else {
        this.lastItemId = ''
        this.lastItemHash = ''
      }
    } catch (error) {
      this.lastItemId = ''
      this.lastItemHash = ''
    }
  }

  public syncDeletedRecords(deletedIds: string[]): void {
    try {
      if (deletedIds.includes('*')) {
        // 清空所有记录
        this.lastItemId = ''
        this.lastItemHash = ''
        this.clearCache()
      } else {
        // 从缓存中移除已删除的记录
        const deletedHashes: string[] = []
        for (const [hash, record] of this.historyCache.entries()) {
          if (deletedIds.includes(record.id)) {
            deletedHashes.push(hash)
          }
        }
        this.removeMultipleFromCache(deletedHashes)

        // 如果当前项被删除，重新加载最后一项信息
        if (deletedIds.includes(this.lastItemId)) {
          this.loadLastItemInfo()
        }
      }

      if (deletedIds.length > 0) {
        this.sendEvent('history-updated', { timestamp: Date.now(), source: 'cleanup' })
      }
    } catch (error) {
      // 静默处理错误
    }
  }

  /**
   * 处理来自清理器的删除通知
   * 接收删除的记录哈希列表，从缓存中移除
   */
  public syncDeletedRecordsByHash(deletedHashes: string[]): void {
    try {
      if (deletedHashes.length === 0) return

      // 检查是否包含当前项
      if (deletedHashes.includes(this.lastItemHash)) {
        this.lastItemId = ''
        this.lastItemHash = ''
        this.loadLastItemInfo()
      }

      // 从缓存中移除
      this.removeMultipleFromCache(deletedHashes)

      this.sendEvent('history-updated', { timestamp: Date.now(), source: 'cleanup' })
    } catch (error) {
      // 静默处理错误
    }
  }

  /**
   * 处理来自清理器的删除通知（优化版本）
   * 接收删除的记录 ID 列表，从缓存中移除
   * 优化：直接使用 ID，无需查询 hash
   */
  public syncDeletedRecordsByIds(deletedIds: string[]): void {
    try {
      if (deletedIds.length === 0) return

      // 检查是否包含当前项
      if (deletedIds.includes(this.lastItemId)) {
        this.lastItemId = ''
        this.lastItemHash = ''
        this.loadLastItemInfo()
      }

      // 从缓存中移除（通过 ID）
      this.removeMultipleFromCacheByIds(deletedIds)

      this.sendEvent('history-updated', { timestamp: Date.now(), source: 'cleanup' })
    } catch (error) {
      // 静默处理错误
    }
  }


  public getConfig(): ClipboardManagerConfig {
    return { ...this.config }
  }

  public updateConfig(newConfig: Partial<ClipboardManagerConfig>): void {
    this.config = { ...this.config, ...newConfig }
  }

  public isListeningActive(): boolean {
    return this.isListening
  }

  public async syncClipboardHash(): Promise<void> {
    try {
      // 直接获取剪贴板数据
      try {
        const clipboardData = this.readClipboardDirect()
        this.lastClipboardHash = await this.generateClipboardHashFromData(clipboardData)
        this.checkQuickClipboardChange(clipboardData)
      } catch (error) {

      }
      this.resetErrorState()
    } catch (error) {
      // 静默处理错误
    }
  }

  public async getCurrentClipboard(): Promise<ClipboardContent> {
    try {
      // 直接获取剪贴板数据
      const clipboardData = this.readClipboardDirect()
      return await this.buildClipboardContentFromData(clipboardData)
    } catch (error) {
      console.error('[剪贴板管理器] 获取当前剪贴板内容失败:', error)
      return {}
    }
  }

  private async getCurrentAppInfo(): Promise<AppInfo | undefined> {
    try {
      // 检查 ActiveWindow 是否已初始化
      if (!this.activeWindowInitialized) {
        return undefined
      }

      const activeWindow = ActiveWindow.getActiveWindow()

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
      return undefined
    }
  }

  public handleMessage(message: WorkerMessage): void {
    if (message.type === 'request') {
      this.handleRequest(message as WorkerRequest)
    }
  }

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
        case 'syncClipboardHash':
          result = await this.syncClipboardHash()
          break
        case 'resetErrorState':
          result = this.resetErrorState()
          break
        case 'syncDeletedRecords':
          result = this.syncDeletedRecords(request.params)
          break
        case 'syncDeletedRecordsByHash':
          result = this.syncDeletedRecordsByHash(request.params)
          break
        case 'syncDeletedRecordsByIds':
          result = this.syncDeletedRecordsByIds(request.params)
          break
        case 'exportHeapdump':
          result = this.exportHeapdump(request.params)
          break
        default:
          throw new Error(`未知方法: ${request.method}`)
      }

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

const worker = new ClipboardManagerWorker()

parentPort?.on('message', (message: WorkerMessage) => {
  worker.handleMessage(message)
})

const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: {},
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
