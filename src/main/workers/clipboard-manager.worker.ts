import { parentPort } from 'node:worker_threads'
import { HashUtils } from '../utils/common'
import { BaseWorker } from './base-worker'
import {
  ClipboardManager as TeddyClipboardManager,
  ClipboardListener,
  ClipboardData
} from '@teddyzhu/clipboard'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../types/worker-types'
import type { ClipboardContent, ClipboardManagerConfig, AppInfo } from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import ActiveWindow from '@paymoapp/active-window'
import logger from '../utils/logger'
import { ocrService, OCRServiceOptions } from '../services/ocr-service'
import { qrCodeService } from '../services/qrcode-service'
import path from 'path'

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
  private ocrInitialized: boolean = false
  private ocrResourcePath: string = ''
  private isDev: boolean = false

  // 历史记录缓存：hash -> { id, hash }
  private historyCache: Map<string, { id: string; hash: string }> = new Map()
  // 反向索引：id -> hash，用于根据 ID 快速查找和删除
  private idToHashIndex: Map<string, string> = new Map()
  private maxCacheSize: number = 10000 // 最大缓存条目数

  constructor() {
    super()
    this.config = {
      maxHistorySize: 100,
      checkInterval: 200,
      enableAutoSave: false,
      enableLocalOCR: false
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
   * 使用最新的 getImageData API 获取完整的图片信息，并支持文件
   */
  private readClipboardDirect(): any {
    try {
      // 安全地获取文本内容
      let text: string | undefined = undefined
      try {
        text = this.clipboard.getText()
      } catch (e) {
        // 文本获取失败时静默处理
      }

      // 安全地获取 HTML 内容
      let html: string | undefined = undefined
      try {
        html = this.clipboard.getHtml()
      } catch (e) {
        // HTML 获取失败时静默处理
      }

      // 安全地获取 RTF 内容
      let rtf: string | undefined = undefined
      try {
        rtf = this.clipboard.getRichText()
      } catch (e) {
        // RTF 获取失败时静默处理
      }

      // 安全地获取文件列表
      let files: string[] | undefined = undefined
      let hasFiles = false
      let filesInfo: { count: number; totalSize: number } | undefined = undefined
      try {
        files = this.clipboard.getFiles()
        hasFiles = !!(files && files.length > 0)
        if (hasFiles) {
          filesInfo = {
            count: files!.length,
            totalSize: files!.reduce((total, file) => total + file.length, 0)
          }
        }
      } catch (e) {
        // 文件获取失败时静默处理
      }

      // 使用最新的 getImageData API 获取完整图片信息
      let imageData: any = null
      let hasImage = false

      try {
        imageData = this.clipboard.getImageData()
        hasImage = !!(imageData && imageData.data && Buffer.isBuffer(imageData.data))
      } catch (e) {
        // 图片获取失败时静默处理
        hasImage = false
      }

      const result: any = {
        text: text || undefined,
        html: html || undefined,
        rtf: rtf || undefined,
        files: files || undefined,
        hasImage: hasImage,
        hasFiles: hasFiles
      }

      // 如果有图片，添加图片的基本信息用于快速变化检测
      if (hasImage && imageData) {
        result.imageInfo = {
          width: imageData.width || 0,
          height: imageData.height || 0,
          quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
        }
        result.image = imageData.data // 现在是 Buffer 数据
      }

      // 如果有文件，添加文件基本信息
      if (hasFiles && filesInfo) {
        result.filesInfo = filesInfo
      }

      return result
    } catch (error) {
      logger.error('[剪贴板管理器 Worker] 直接读取剪贴板失败:', error)
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
          logger.warn('[剪贴板管理器 Worker] macOS 需要屏幕录制权限才能获取窗口标题')
        }
      }
      this.activeWindowInitialized = true
      logger.info('[剪贴板管理器 Worker] ActiveWindow 初始化成功')
    } catch (error) {
      logger.error('[剪贴板管理器 Worker] ActiveWindow 初始化失败:', error)
      this.activeWindowInitialized = false
    }
  }

  /**
   * 设置 OCR 相关参数
   */
  private setupOCRParameters(isPackaged?: boolean, appPath?: string): void {
    try {
      // 判断是否为开发环境
      this.isDev = false

      // 根据是否打包来确定资源路径
      logger.info('[剪贴板管理器 Worker] 是否为打包环境:', isPackaged)
      if (isPackaged) {
        // 打包后的路径 - 使用程序安装目录下的 resources
        this.ocrResourcePath = path.join(appPath || '', 'resources', 'ocr').replace('/app.asar', '')
        logger.info('[剪贴板管理器 Worker] 打包环境下的 OCR 资源路径:', this.ocrResourcePath)
      } else {
        // 开发环境路径 - 从当前文件位置向上查找
        this.ocrResourcePath = path.join(__dirname, '..', '..', 'resources', 'ocr')
      }

      logger.debug('[剪贴板管理器 Worker] OCR 参数设置完成:', {
        isDev: this.isDev,
        resourcePath: this.ocrResourcePath
      })
    } catch (error) {
      logger.error('[剪贴板管理器 Worker] OCR 参数设置失败:', error)
      // 设置默认值
      this.isDev = false
      this.ocrResourcePath = ''
    }
  }

  /**
   * 初始化 OCR 服务
   */
  private async initializeOCRService(): Promise<void> {
    if (this.ocrInitialized) {
      return
    }

    if (!this.ocrResourcePath) {
      throw new Error('OCR 资源路径未设置')
    }

    const ocrOptions: OCRServiceOptions = {
      resourcePath: this.ocrResourcePath,
      isDev: this.isDev,
      ocrConfig: {
        detRatio: 1.0,
        recImgHeight: 48,
        enableSpaceOptimize: true,
        enableDocCls: false
      }
    }

    await ocrService.initialize(ocrOptions)
    this.ocrInitialized = true

    logger.debug('[剪贴板管理器 Worker] OCR 服务初始化成功')
  }

  public async initialize(
    params?:
      | {
          config?: Partial<ClipboardManagerConfig>
          userDataPath?: string
          isPackaged?: boolean
          appPath?: string
        }
      | Partial<ClipboardManagerConfig>
  ): Promise<void> {
    // 处理参数兼容性
    let config: Partial<ClipboardManagerConfig> | undefined
    let userDataPath: string | undefined
    let isPackaged: boolean = true
    let appPath: string | undefined
    if (params && 'config' in params) {
      // 新格式：{ config, userDataPath, isPackaged, appPath }
      config = params.config
      userDataPath = params.userDataPath
      isPackaged = params.isPackaged || true
      appPath = params.appPath
    } else {
      // 旧格式：直接传配置对象
      config = params as Partial<ClipboardManagerConfig>
    }

    // 初始化 Worker 基础功能（包括日志系统）
    if (userDataPath) {
      this.initializeWorker({
        userDataPath,
        logFileName: 'clipboard-manager-worker.log',
        logScope: 'clipboard-manager-worker'
      })
    }

    if (config) {
      this.config = { ...this.config, ...config }
    }

    // 设置 OCR 相关参数
    this.setupOCRParameters(isPackaged, appPath)

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
      logger.error('[剪贴板管理器 Worker] 初始化失败:', error)
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

      logger.debug('[剪贴板管理器 Worker] 剪贴板变化检测:', new Date().toDateString())
      logger.debug('[剪贴板管理器 Worker] 可用格式:', data.availableFormats)

      // 将 ClipboardData 转换为内部格式
      const clipboardData = this.convertClipboardData(data)

      // 检查快速变化（基于文本、HTML、RTF、书签和图片存在性）
      const hasQuickChange = this.checkQuickClipboardChange(clipboardData)
      if (!hasQuickChange) {
        this.onOperationSuccess()
        return
      }
      logger.debug(
        '[剪贴板管理器 Worker] 快速检查失败，进行完整哈希检查:',
        new Date().toDateString()
      )

      // 生成完整内容哈希
      const currentHash = await this.generateClipboardHashFromData(clipboardData)
      logger.debug('[剪贴板管理器 Worker] 完整哈希:', currentHash)
      if (currentHash && currentHash !== this.lastClipboardHash) {
        this.lastClipboardHash = currentHash

        if (this.config.enableAutoSave) {
          // 从已获取的数据构建完整内容
          const clipboardContent = await this.buildClipboardContentFromData(clipboardData)
          await this.saveClipboardItem(clipboardContent)
          logger.debug('[剪贴板管理器 Worker] 剪贴板项目保存完成:', new Date().toDateString())
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
      hasImage: !!data.image,
      hasFiles: !!(data.files && data.files.length > 0)
    }

    // 处理文件数据
    if (data.files && data.files.length > 0) {
      result.files = data.files
      result.filesInfo = {
        count: data.files.length,
        totalSize: data.files.reduce((total, file) => total + file.length, 0)
      }
    }

    // 处理图片数据 - 使用最新的 ImageData 结构
    if (data.image) {
      const imageData = data.image as any // ImageData 接口包含 width, height, size, data
      result.imageInfo = {
        width: imageData.width || 0,
        height: imageData.height || 0,
        quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
      }
      result.image = imageData.data // 现在是 Buffer 数据
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
        hasImage: clipboardData.hasImage,
        hasFiles: clipboardData.hasFiles,
        // 包含图片的基本信息用于区分不同的图片
        imageInfo: clipboardData.imageInfo || undefined,
        // 包含文件的基本信息用于区分不同的文件列表
        filesInfo: clipboardData.filesInfo || undefined
      }
      if (quickContent.hasImage) {
        logger.debug('[剪贴板管理器 Worker] 检测到图片:', quickContent.imageInfo)
      }
      const quickHash = this.generateQuickHash(quickContent)
      if (quickContent.hasImage) {
        logger.debug('[剪贴板管理器 Worker] 快速哈希变化:', quickHash, this.lastQuickHash)
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
          files: clipboardData.files || undefined,
          image: clipboardData.image
        }
      }

      // 没有图片，直接构建内容
      return {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        files: clipboardData.files || undefined
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
        // console.log('apps ',sourceAppInfo)
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
          sourceApp: sourceAppId,
          hash: contentHash
        })

        // 添加到缓存
        this.addToCache(newItemId, contentHash)

        this.lastItemId = newItemId
        this.lastItemHash = contentHash

        // 如果启用了本地OCR且有图片，异步进行OCR识别
        if (this.config.enableLocalOCR && content.image) {
          this.performAsyncOCR(newItemId, content.image)
        }

        // check in clean workers
        // this.sendEvent('cleanup-check-required', {
        //   reason: 'new-record-added',
        //   maxHistorySize: this.config.maxHistorySize
        // })
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
  private getRecordFromCache(hash: string): { id: string; hash: string } | null {
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
      } catch (error) {}
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
      logger.error('[剪贴板管理器 Worker] 获取当前剪贴板内容失败:', error)
      return {}
    }
  }

  // ==================== 剪贴板还原方法 ====================

  /**
   * 还原剪贴板内容（所有格式）
   * 直接使用 setContents API 一次性设置所有格式
   */
  public restoreClipboardContent(params: { content: ClipboardContent }): boolean {
    try {
      // 参数验证
      if (!params || !params.content) {
        console.error('[剪贴板管理器 Worker] 还原失败: 无效的参数')
        return false
      }

      const content = params.content

      // 构建 ClipboardData 对象，根据 available formats 设置所有数据
      const clipboardData: any = {
        availableFormats: [] as string[]
      }

      // 添加文本内容
      if (content.text) {
        clipboardData.text = content.text
        clipboardData.availableFormats.push('text')
        console.log('[剪贴板管理器 Worker] 添加文本内容')
      }

      // 添加 HTML 内容
      if (content.html) {
        clipboardData.html = content.html
        clipboardData.availableFormats.push('html')
        console.log('[剪贴板管理器 Worker] 添加 HTML 内容')
      }

      // 添加 RTF 内容
      if (content.rtf) {
        clipboardData.rtf = content.rtf
        clipboardData.availableFormats.push('rtf')
        console.log('[剪贴板管理器 Worker] 添加 RTF 内容')
      }

      // 添加图片内容
      if (content.image) {
        try {
          // 构建 ImageData 对象，使用 Buffer 数据
          clipboardData.image = {
            width: 0, // 宽度信息可能不可用，设为0
            height: 0, // 高度信息可能不可用，设为0
            size: content.image.length,
            data: content.image // 现在使用 Buffer 数据
          }
          clipboardData.availableFormats.push('image')
          console.log('[剪贴板管理器 Worker] 添加图片内容')
        } catch (e) {
          console.warn('[剪贴板管理器 Worker] 处理图片数据失败:', e)
        }
      }

      // 添加文件内容
      if (content.files && content.files.length > 0) {
        try {
          clipboardData.files = content.files
          clipboardData.availableFormats.push('files')
          console.log('[剪贴板管理器 Worker] 添加文件内容:', content.files.length, '个文件')
        } catch (e) {
          console.warn('[剪贴板管理器 Worker] 处理文件数据失败:', e)
        }
      }

      // 检查是否有内容可以设置
      if (clipboardData.availableFormats.length === 0) {
        console.warn('[剪贴板管理器 Worker] 警告: 没有有效的剪贴板内容可以还原')
        return false
      }

      // 直接使用 setContents API 设置剪贴板内容
      this.clipboard.setContents(clipboardData)
      console.log(
        '[剪贴板管理器 Worker] 已使用 setContents 设置剪贴板内容，格式:',
        clipboardData.availableFormats
      )

      return true
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 还原剪贴板时发生异常:', error)
      return false
    }
  }

  /**
   * 还原剪贴板内容为纯文本
   * 直接使用 setContents API 确保只设置文本格式
   * 当不存在 text 时，将其他可转换的类型转为 text
   */
  public restoreClipboardContentAsText(params: { content: ClipboardContent }): boolean {
    try {
      // 参数验证
      if (!params || !params.content) {
        console.error('[剪贴板管理器 Worker] 还原为纯文本失败: 无效的参数')
        return false
      }

      const content = params.content
      let textContent = ''

      // 优先使用原有的文本内容
      if (content.text && content.text.trim()) {
        textContent = content.text
      }
      // 如果没有文本内容，尝试从其他格式转换
      else {
        textContent = this.convertToText(content)

        if (!textContent || !textContent.trim()) {
          console.warn('[剪贴板管理器 Worker] 内容中没有可转换为文本的数据')
          return false
        }
      }

      // 直接使用 setContents API 只设置文本格式
      const clipboardData = {
        availableFormats: ['text'],
        text: textContent
      }

      this.clipboard.setContents(clipboardData)
      console.log('[剪贴板管理器 Worker] 已使用 setContents 设置纯文本内容')

      return true
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 还原剪贴板为纯文本失败:', error)
      return false
    }
  }

  /**
   * 将其他格式的内容转换为纯文本
   * 按优先级顺序尝试转换：HTML -> RTF -> Files
   * 注意：不支持图片转文本
   */
  private convertToText(content: ClipboardContent): string {
    // 1. 尝试从 HTML 转换
    if (content.html && content.html.trim()) {
      try {
        // 移除 HTML 标签，保留文本内容
        const textFromHtml = content.html
          .replace(/<script[^>]*>[\s\S]*?<\/script>/gi, '') // 移除 script 标签及内容
          .replace(/<style[^>]*>[\s\S]*?<\/style>/gi, '') // 移除 style 标签及内容
          .replace(/<[^>]*>/g, '') // 移除所有 HTML 标签
          .replace(/&nbsp;/g, ' ') // 替换 HTML 空格实体
          .replace(/&lt;/g, '<') // 替换 HTML 实体
          .replace(/&gt;/g, '>')
          .replace(/&amp;/g, '&')
          .replace(/&quot;/g, '"')
          .replace(/&#39;/g, "'")
          .replace(/\s+/g, ' ') // 合并多个空白字符
          .trim()

        if (textFromHtml) {
          console.log('[剪贴板管理器 Worker] 从 HTML 内容转换为纯文本')
          return textFromHtml
        }
      } catch (error) {
        console.warn('[剪贴板管理器 Worker] HTML 转文本失败:', error)
      }
    }

    // 2. 尝试从 RTF 转换
    if (content.rtf && content.rtf.trim()) {
      try {
        // 简单的 RTF 转文本：移除 RTF 控制字符和格式代码
        const textFromRtf = content.rtf
          .replace(/\\[a-z]+\d*\s?/g, '') // 移除 RTF 控制字符
          .replace(/[{}]/g, '') // 移除大括号
          .replace(/\\\\/g, '\\') // 处理转义的反斜杠
          .replace(/\\'/g, "'") // 处理转义的单引号
          .replace(/\s+/g, ' ') // 合并多个空白字符
          .trim()

        if (textFromRtf) {
          console.log('[剪贴板管理器 Worker] 从 RTF 内容转换为纯文本')
          return textFromRtf
        }
      } catch (error) {
        console.warn('[剪贴板管理器 Worker] RTF 转文本失败:', error)
      }
    }

    // 3. 尝试从文件列表转换
    if (content.files && content.files.length > 0) {
      try {
        const fileList = content.files.join('\n')
        if (fileList) {
          console.log('[剪贴板管理器 Worker] 从文件列表转换为纯文本')
          return fileList
        }
      } catch (error) {
        console.warn('[剪贴板管理器 Worker] 文件列表转文本失败:', error)
      }
    }

    // 图片不支持转换为文本
    return ''
  }

  /**
   * 从数据库获取内容并还原剪贴板（完整还原）
   */
  public async restoreClipboardFromDatabase(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        return { success: false, message: '无效的项目ID' }
      }

      // 从数据库获取记录
      const targetRecord = await this.callMainThread('database-getRecordByContentId', {
        contentId: itemId
      })

      if (!targetRecord) {
        return { success: false, message: '未找到指定的剪贴板项' }
      }

      // 重建内容对象
      const content: ClipboardContent = {}
      if (targetRecord.content_text) content.text = targetRecord.content_text
      if (targetRecord.content_html) content.html = targetRecord.content_html
      if (targetRecord.content_image) content.image = targetRecord.content_image
      if (targetRecord.content_rtf) content.rtf = targetRecord.content_rtf
      if (targetRecord.content_files) {
        try {
          content.files = JSON.parse(targetRecord.content_files)
        } catch {
          content.files = []
        }
      }

      // 使用 restoreClipboardContent 方法还原所有格式
      const success = this.restoreClipboardContent({ content })

      if (success) {
        // 更新最后使用时间
        const now = Date.now()
        await this.callMainThread('database-updateLastUsed', {
          contentId: itemId,
          lastUsed: now
        })
      }

      return {
        success,
        message: success ? '剪贴板内容已还原' : '还原失败'
      }
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 从数据库还原剪贴板失败:', error)
      return { success: false, message: '还原剪贴板失败' }
    }
  }

  /**
   * 从数据库获取内容并还原剪贴板为纯文本
   * 支持从其他格式转换为文本
   */
  public async restoreClipboardFromDatabaseAsText(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        return { success: false, message: '无效的项目ID' }
      }

      // 从数据库获取记录
      const targetRecord = await this.callMainThread('database-getRecordByContentId', {
        contentId: itemId
      })

      if (!targetRecord) {
        return { success: false, message: '未找到指定的剪贴板项' }
      }

      // 重建完整的内容对象，包含所有可用格式
      const content: ClipboardContent = {}
      if (targetRecord.content_text) content.text = targetRecord.content_text
      if (targetRecord.content_html) content.html = targetRecord.content_html
      if (targetRecord.content_rtf) content.rtf = targetRecord.content_rtf
      if (targetRecord.content_image) content.image = targetRecord.content_image
      if (targetRecord.content_files) {
        try {
          content.files = JSON.parse(targetRecord.content_files)
        } catch {
          content.files = []
        }
      }

      // 使用优化后的 restoreClipboardContentAsText 方法
      // 它会自动尝试从其他格式转换为文本
      const success = this.restoreClipboardContentAsText({ content })

      if (success) {
        // 更新最后使用时间
        const now = Date.now()
        await this.callMainThread('database-updateLastUsed', {
          contentId: itemId,
          lastUsed: now
        })
      }

      return {
        success,
        message: success ? '剪贴板内容已还原为纯文本' : '还原失败'
      }
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 从数据库还原剪贴板为纯文本失败:', error)
      return { success: false, message: '还原剪贴板为纯文本失败' }
    }
  }

  /**
   * 从数据库获取内容并还原剪贴板为OCR文本
   * 专门用于图片的OCR识别文本
   */
  public async restoreClipboardFromDatabaseAsOcrText(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        return { success: false, message: '无效的项目ID' }
      }

      // 从数据库获取记录
      const targetRecord = await this.callMainThread('database-getRecordByContentId', {
        contentId: itemId
      })

      if (!targetRecord) {
        return { success: false, message: '未找到指定的剪贴板项' }
      }

      // 检查是否有OCR文本内容
      if (!targetRecord.content_ocr || !targetRecord.content_ocr.trim()) {
        return { success: false, message: '该项目没有OCR文本内容' }
      }

      // 直接使用 setContents API 只设置OCR文本
      const clipboardData = {
        availableFormats: ['text'],
        text: targetRecord.content_ocr
      }
      this.clipboard.setContents(clipboardData)

      console.log('[剪贴板管理器 Worker] OCR文本已还原到剪贴板')

      return {
        success: true,
        message: 'OCR文本已还原到剪贴板'
      }
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 从数据库还原剪贴板为OCR文本失败:', error)
      return { success: false, message: '还原剪贴板为OCR文本失败' }
    }
  }

  /**
   * 解析剪贴板项目中的二维码并复制到剪贴板
   * @param itemId 剪贴板项目ID
   */
  public async parseQRCodeFromItem(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        return { success: false, message: '无效的项目ID' }
      }

      // 从数据库获取记录
      const targetRecord = await this.callMainThread('database-getRecordByContentId', {
        contentId: itemId
      })
      if (!targetRecord) {
        return { success: false, message: '未找到指定的剪贴板项' }
      }

      // 检查是否有图片内容
      if (!targetRecord.content_image) {
        return { success: false, message: '该项目不是图片类型' }
      }

      try {
        // 将 base64 图片数据转换为 Buffer
        const imageBuffer = Buffer.from(targetRecord.content_image, 'base64')

        // 使用二维码服务解析图片
        const qrText = await qrCodeService.parseFromBuffer(imageBuffer)

        if (qrText && qrText.trim()) {
          // 使用 TeddyClipboardManager 设置剪贴板内容
          const clipboardData = {
            availableFormats: ['text'],
            text: qrText
          }
          this.clipboard.setContents(clipboardData)

          console.log('[剪贴板管理器 Worker] 二维码解析成功并已复制到剪贴板')
          return {
            success: true,
            message: '二维码解析成功，内容已复制到剪贴板'
          }
        } else {
          return {
            success: false,
            message: '图片中未发现二维码或解析失败'
          }
        }
      } catch (error) {
        console.error('[剪贴板管理器 Worker] 二维码解析过程出错:', error)
        return { success: false, message: '二维码解析过程出错' }
      }
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 解析二维码失败:', error)
      return { success: false, message: '解析二维码失败' }
    }
  }

  /**
   * 设置图片到剪贴板
   * @param imageBuffer 图片Buffer数据
   */
  public setImageToClipboard(imageBuffer: Buffer | Uint8Array | any): boolean {
    try {
      console.log('[剪贴板管理器 Worker] 收到图片数据:', {
        isBuffer: Buffer.isBuffer(imageBuffer),
        isUint8Array: imageBuffer instanceof Uint8Array,
        length: imageBuffer?.length,
        type: typeof imageBuffer,
        constructor: imageBuffer?.constructor?.name
      })

      // 参数验证 - 检查是否为Buffer、Uint8Array或者可以转换为Buffer
      let buffer: Buffer
      if (Buffer.isBuffer(imageBuffer)) {
        buffer = imageBuffer
      } else if (imageBuffer instanceof Uint8Array) {
        // Worker线程间通信时Buffer会被序列化为Uint8Array
        buffer = Buffer.from(imageBuffer)
        console.log('[剪贴板管理器 Worker] 从Uint8Array转换为Buffer，大小:', buffer.length)
      } else if (Array.isArray(imageBuffer)) {
        // 处理普通数组的情况
        buffer = Buffer.from(imageBuffer)
      } else if (imageBuffer && typeof imageBuffer === 'object' && (imageBuffer as any).data) {
        // 如果是序列化后的Buffer对象，尝试恢复
        buffer = Buffer.from((imageBuffer as any).data)
      } else {
        console.error('[剪贴板管理器 Worker] 无效的图片数据:', typeof imageBuffer, imageBuffer)
        return false
      }

      if (buffer.length === 0) {
        console.error('[剪贴板管理器 Worker] 图片数据为空')
        return false
      }

      // 使用 TeddyClipboardManager 设置图片内容
      const clipboardData = {
        availableFormats: ['image'],
        image: {
          data: buffer,
          width: 0,
          height: 0,
          size: buffer.length
        }
      }
      this.clipboard.setContents(clipboardData)

      console.log('[剪贴板管理器 Worker] 图片已设置到剪贴板，大小:', buffer.length)
      return true
    } catch (error) {
      console.error('[剪贴板管理器 Worker] 设置图片到剪贴板失败:', error)
      return false
    }
  }

  /**
   * 异步执行OCR识别
   */
  private async performAsyncOCR(contentId: string, imageBuffer: Buffer): Promise<void> {
    try {
      logger.debug('[剪贴板管理器 Worker] 开始异步OCR识别:', contentId)

      // 确保OCR服务已初始化
      if (!ocrService.ready) {
        try {
          await this.initializeOCRService()
        } catch (error) {
          logger.error('[剪贴板管理器 Worker] OCR服务初始化失败:', error)
          return
        }
      }

      // 执行OCR识别
      const ocrResult = await ocrService.recognizeText(imageBuffer)

      if (ocrResult.success && ocrResult.text && ocrResult.text.trim()) {
        logger.debug(
          '[剪贴板管理器 Worker] OCR识别成功:',
          contentId,
          '识别文本长度:',
          ocrResult.text.length
        )

        // 更新数据库中的OCR字段
        const updateSuccess = await this.callMainThread('database-updateOCR', {
          contentId: contentId,
          ocrText: ocrResult.text
        })

        if (updateSuccess) {
          // 通知前端更新这条记录的OCR值
          this.sendEvent('record-ocr-updated', {
            contentId: contentId,
            ocrText: ocrResult.text,
            timestamp: Date.now()
          })
          logger.debug('[剪贴板管理器 Worker] OCR结果已更新到数据库并通知前端:', contentId)
        } else {
          logger.error('[剪贴板管理器 Worker] 更新OCR结果到数据库失败:', contentId)
        }
      } else {
        logger.debug('[剪贴板管理器 Worker] OCR识别失败或无文本:', contentId, ocrResult.error)
      }
    } catch (error) {
      logger.error('[剪贴板管理器 Worker] 异步OCR处理失败:', contentId, error)
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
        case 'restoreClipboardContent':
          result = this.restoreClipboardContent(request.params)
          break
        case 'restoreClipboardContentAsText':
          result = this.restoreClipboardContentAsText(request.params)
          break
        case 'restoreClipboardFromDatabase':
          result = await this.restoreClipboardFromDatabase(request.params)
          break
        case 'restoreClipboardFromDatabaseAsText':
          result = await this.restoreClipboardFromDatabaseAsText(request.params)
          break
        case 'restoreClipboardFromDatabaseAsOcrText':
          result = await this.restoreClipboardFromDatabaseAsOcrText(request.params)
          break
        case 'parseQRCodeFromItem':
          result = await this.parseQRCodeFromItem(request.params)
          break
        case 'setImageToClipboard':
          result = this.setImageToClipboard(request.params)
          break
        case 'getCurrentClipboard':
          result = await this.getCurrentClipboard()
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

// 发送 worker-ready 消息（统一发送时机）
const readyEvent: WorkerEvent = {
  id: `event-${Date.now()}-${Math.random()}`,
  type: 'event',
  event: 'worker-ready',
  data: { workerName: '剪贴板管理器 Worker' },
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
