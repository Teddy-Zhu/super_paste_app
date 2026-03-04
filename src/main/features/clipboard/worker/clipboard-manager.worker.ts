import { parentPort } from 'node:worker_threads'
import { BaseWorker } from '../../../workers/base-worker'
import { ClipboardData } from '@teddyzhu/clipboard'
import type {
  WorkerMessage,
  WorkerRequest,
  WorkerResponse,
  WorkerEvent
} from '../../../types/worker-types'
import type { ClipboardContent, ClipboardManagerConfig, AppInfo } from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import logger from '../../../utils/worker-logger'
import { qrCodeService } from '../../../services/qrcode-service'
import path from 'path'
import fs from 'fs'
import { ocrService } from '../../../services/ocr-service'
import type { OCRServiceOptions } from '../../../services/ocr-service'
// Modules
import { ClipboardPoller } from './modules/clipboard-poller'
import { ContentProcessor } from './modules/content-processor'
import { ClipboardRepository } from './modules/clipboard-repository'

class ClipboardManagerWorker extends BaseWorker {
  private config: ClipboardManagerConfig

  // Modules
  private poller: ClipboardPoller
  private processor: ContentProcessor
  private repository: ClipboardRepository

  // State
  private activeWindowInitialized: boolean = false
  private cachedActiveWindow: any | null = null
  private ocrResourcePath: string = ''
  private ocrIsDev: boolean = false
  private ocrInitializing: Promise<boolean> | null = null
  private ocrConfigHash: string = ''

  constructor() {
    super()
    this.config = {
      maxHistorySize: 100,
      enableAutoSave: false,
      enableLocalOCR: false,
      ocrMode: 'local',
      ocrRemoteConfig: {
        url: '',
        extraParams: '{}',
        imageFieldName: 'image',
        requestFormat: 'json',
        resultPath: 'data.result'
      }
    }

    this.poller = new ClipboardPoller()
    this.processor = new ContentProcessor()
    this.repository = new ClipboardRepository()
  }

  protected getWorkerName(): string {
    return '剪贴板变化检测 Worker'
  }

  public async initialize(params?: any): Promise<void> {
    // 处理参数兼容性
    let config: Partial<ClipboardManagerConfig> | undefined
    let userDataPath: string | undefined
    let isPackaged: boolean = true
    let appPath: string | undefined
    if (params && 'config' in params) {
      config = params.config
      userDataPath = params.userDataPath
      isPackaged = params.isPackaged ?? true
      appPath = params.appPath
    } else {
      config = params as Partial<ClipboardManagerConfig>
    }

    if (userDataPath) {
      this.initializeWorker({
        userDataPath,
        logFileName: 'clipboard-manager-worker.log',
        logScope: 'clipboard-manager-worker'
      })
      this.repository.initialize({ userDataPath, maxCacheSize: 10000 })
    }

    if (config) {
      this.config = {
        ...this.config,
        ...config,
        ocrRemoteConfig: {
          ...this.config.ocrRemoteConfig,
          ...(config.ocrRemoteConfig || {})
        }
      }
    }

    // 设置 OCR 相关参数
    this.setupOCRParameters(isPackaged, appPath)
    if (this.config.enableLocalOCR) {
      await this.ensureOCRServiceInitialized()
    }

    // 初始化时进行一次全量检查
    try {
      const rawData = this.poller.readClipboardDirect()
      // logger.info('[ClipboardManagerWorker] Initial check rawData:', rawData)
      const converted = this.processor.convertClipboardData(rawData)
      await this.processor.buildClipboardContent(converted)
      // Store current hash in processor/repo context passed if needed, 
      // but modules handle their own state mostly. 
      // Sync logic might need to align last hash.
      // Actually Repository maintains lastItemHash.
      // But Processor maintains lastQuickHash.
      this.processor.checkQuickClipboardChange(converted)

      // Ensure repository has loaded last item info
      // (It does in initialize)
    } catch (error) {
      logger.error('[ClipboardManagerWorker] Initial check failed:', error)
    }
  }

  private setupOCRParameters(isPackaged?: boolean, appPath?: string): void {
    try {
      logger.info('[剪贴板管理器 Worker] 是否为打包环境:', isPackaged)
      this.ocrIsDev = !isPackaged
      this.ocrResourcePath = this.resolveOCRResourcePath(Boolean(isPackaged), appPath)

      if (this.ocrResourcePath) {
        logger.info('[剪贴板管理器 Worker] OCR 资源路径:', this.ocrResourcePath)
      } else {
        logger.warn('[剪贴板管理器 Worker] 未找到可用的 OCR 资源路径')
      }
    } catch (error) {
      logger.error('[剪贴板管理器 Worker] OCR 参数设置失败:', error)
      this.ocrResourcePath = ''
      this.ocrIsDev = false
    }
  }

  private resolveOCRResourcePath(isPackaged: boolean, appPath?: string): string {
    const normalizedAppPath = (appPath || '').replace(/[\\/]app\.asar$/, '')
    const candidates = isPackaged
      ? [
        path.join(process.resourcesPath || '', 'resources', 'ocr'),
        path.join(process.resourcesPath || '', 'ocr'),
        path.join(normalizedAppPath, 'resources', 'ocr'),
        path.join(normalizedAppPath, 'ocr')
      ]
      : [
        path.resolve(process.cwd(), 'resources', 'ocr'),
        path.resolve(normalizedAppPath, 'resources', 'ocr'),
        path.resolve(__dirname, '..', '..', '..', '..', '..', 'resources', 'ocr')
      ]

    for (const candidate of candidates) {
      if (this.isValidOCRResourcePath(candidate)) {
        return candidate
      }
    }

    logger.warn('[剪贴板管理器 Worker] OCR 资源路径候选均不可用:', candidates)
    return ''
  }

  private isValidOCRResourcePath(resourcePath: string): boolean {
    if (!resourcePath) {
      return false
    }
    const requiredFiles = ['ppocr_det.onnx', 'ppocr_rec.onnx', 'ppocr_keys_v1.txt']
    return requiredFiles.every((fileName) => fs.existsSync(path.join(resourcePath, fileName)))
  }

  private buildOCRServiceOptions(): OCRServiceOptions | null {
    if (this.config.ocrMode === 'local') {
      if (!this.ocrResourcePath) {
        logger.warn('[剪贴板管理器 Worker] OCR 初始化失败：资源路径为空')
        return null
      }

      return {
        mode: 'local',
        resourcePath: this.ocrResourcePath,
        isDev: this.ocrIsDev
      }
    }

    return {
      mode: 'remote',
      remoteConfig: {
        url: this.config.ocrRemoteConfig.url,
        extraParams: this.config.ocrRemoteConfig.extraParams,
        imageFieldName: this.config.ocrRemoteConfig.imageFieldName,
        requestFormat: this.config.ocrRemoteConfig.requestFormat,
        resultPath: this.config.ocrRemoteConfig.resultPath
      }
    }
  }

  private buildOCRConfigHash(): string {
    return JSON.stringify({
      mode: this.config.ocrMode,
      resourcePath: this.ocrResourcePath,
      isDev: this.ocrIsDev,
      remoteConfig: this.config.ocrRemoteConfig
    })
  }

  private async ensureOCRServiceInitialized(forceReinitialize = false): Promise<boolean> {
    const options = this.buildOCRServiceOptions()
    if (!options) {
      return false
    }

    const nextHash = this.buildOCRConfigHash()
    if (!forceReinitialize && ocrService.ready && this.ocrConfigHash === nextHash) {
      return true
    }

    if (this.ocrInitializing) {
      return this.ocrInitializing
    }

    this.ocrInitializing = (async () => {
      try {
        if (ocrService.ready) {
          await ocrService.reinitialize(options)
        } else {
          await ocrService.initialize(options)
        }
        this.ocrConfigHash = nextHash
        logger.info('[剪贴板管理器 Worker] OCR 服务初始化完成')
        return true
      } catch (error) {
        logger.error('[剪贴板管理器 Worker] OCR 服务初始化失败:', error)
        return false
      } finally {
        this.ocrInitializing = null
      }
    })()

    return this.ocrInitializing
  }

  public startListening(): void {
    this.poller.startListening(this.handleClipboardChange.bind(this))
  }

  public stopListening(): void {
    this.poller.stopListening()
  }

  private async handleClipboardChange(data: ClipboardData): Promise<void> {
    try {
      // console.log('handleClipboardChange', data)
      if (this.isInErrorBackoff()) return

      const converted = this.processor.convertClipboardData(data)

      // Quick check
      const hasQuickChange = this.processor.checkQuickClipboardChange(converted)
      if (!hasQuickChange) {
        this.onOperationSuccess()
        return
      }

      // Full process
      const content = await this.processor.buildClipboardContent(converted)
      const hash = ClipboardDataConverter.generateContentHash(content) // or processor.generateClipboardHash(converted)?
      // processor.generateClipboardHash does buildClipboardContent then hash.
      // but we need 'content' to save it. 

      // Check if repo has it (optimization to avoid building content if we could allow repo to just check hash?)
      // But repo needs content to insert if new.

      if (this.config.enableAutoSave) {
        const appInfo = await this.getCurrentAppInfo()
        const result = await this.repository.saveItem(content, hash, {
          appInfo,
          enableLocalOCR: this.config.enableLocalOCR
        })

        if (result.isNew || result.id !== this.getLastItemId()) { // Logic check
          this.sendEvent('clipboard-changed', { timestamp: Date.now() })
          if (result.saved) {
            this.sendEvent('history-updated', { timestamp: Date.now() })
          }
        }
      } else {
        // Not auto saving, but we should probably detect change to notify frontend?
        // Original logic: if (currentHash !== this.lastClipboardHash) -> notify
        // Repo maintains lastItemHash.
        if (hash !== this.repository.getLastItemHash()) {
          this.sendEvent('clipboard-changed', { timestamp: Date.now() })
        }
      }

      this.onOperationSuccess()
    } catch (error) {
      this.handleCheckError(error, 'Clipboard Check')
    }
  }

  // Active Window Helper
  public updateActiveWindow(windowInfo: any): void {
    this.cachedActiveWindow = windowInfo
    this.activeWindowInitialized = true
  }

  private async getCurrentAppInfo(): Promise<AppInfo | undefined> {
    try {
      if (!this.activeWindowInitialized || !this.cachedActiveWindow) return undefined
      const activeWindow = this.cachedActiveWindow
      if (!activeWindow.application || !activeWindow.path) return undefined

      return {
        id: ClipboardDataConverter.generateAppId(),
        application: activeWindow.application,
        path: activeWindow.path,
        icon: activeWindow.icon,
        createdAt: Date.now(),
        updatedAt: Date.now()
      }
    } catch {
      return undefined
    }
  }

  // Delegate Methods
  public getConfig(): ClipboardManagerConfig {
    return { ...this.config }
  }

  public async updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    const prevConfigHash = this.buildOCRConfigHash()
    const prevEnableLocalOCR = this.config.enableLocalOCR
    this.config = {
      ...this.config,
      ...newConfig,
      ocrRemoteConfig: {
        ...this.config.ocrRemoteConfig,
        ...(newConfig.ocrRemoteConfig || {})
      }
    }
    const hasOCRConfigChanged = prevConfigHash !== this.buildOCRConfigHash()

    if (!prevEnableLocalOCR && this.config.enableLocalOCR) {
      await this.ensureOCRServiceInitialized()
      return
    }

    if (prevEnableLocalOCR && !this.config.enableLocalOCR) {
      ocrService.stop()
      this.ocrConfigHash = ''
      return
    }

    if (this.config.enableLocalOCR && hasOCRConfigChanged) {
      await this.ensureOCRServiceInitialized()
    }
  }

  public isListeningActive(): boolean {
    return this.poller.isListeningActive()
  }

  public async syncClipboardHash(): Promise<void> {
    try {
      const raw = this.poller.readClipboardDirect()
      const converted = this.processor.convertClipboardData(raw)
      await this.processor.generateClipboardHash(converted)
      // We might want to update repo's 'lastItemHash' without saving?
      // Or just update internal state (which is now in repo).
      // If we want to align, we might need a method in repo `setLastItemHash`?
      // Or just let it be.
      this.processor.checkQuickClipboardChange(converted)
    } catch (e) { }
    this.resetErrorState()
  }

  public async getCurrentClipboard(): Promise<ClipboardContent> {
    const raw = this.poller.readClipboardDirect()
    return this.processor.buildClipboardContent(this.processor.convertClipboardData(raw))
  }

  // Restore Methods
  public restoreClipboardContent(params: { content: ClipboardContent }): boolean {
    try {
      if (!params?.content) return false
      const data = this.processor.prepareForRestore(params.content)
      if (data.availableFormats.length === 0) return false

      this.poller.setContents(data)
      return true
    } catch (e) {
      logger.error('[ClipboardManagerWorker] Restore failed:', e)
      return false
    }
  }

  public restoreClipboardContentAsText(params: { content: ClipboardContent }): boolean {
    try {
      if (!params?.content) return false
      const text = this.processor.convertToText(params.content)
      if (!text) return false

      this.poller.setContents({
        availableFormats: ['text'],
        text
      })
      return true
    } catch (e) {
      return false
    }
  }

  public async restoreClipboardFromDatabase(itemId: string): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record) return { success: false, message: 'Item not found' }

    const item = ClipboardDataConverter.recordToItem(record)
    const success = this.restoreClipboardContent({ content: item.content as ClipboardContent })

    if (success) {
      // Update last used
      // We can call saveItem with same content? Or manual update?
      // Repo handles updateLastUsed inside saveItem if hash matches
      const hash = ClipboardDataConverter.generateContentHash(item.content as ClipboardContent)
      this.repository.saveItem(item.content as ClipboardContent, hash)
    }
    return { success, message: success ? 'Restored' : 'Failed' }
  }

  public async restoreClipboardFromDatabaseAsText(itemId: string): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record) return { success: false, message: 'Item not found' }

    const item = ClipboardDataConverter.recordToItem(record)
    const success = this.restoreClipboardContentAsText({ content: item.content as ClipboardContent })

    if (success) {
      const hash = ClipboardDataConverter.generateContentHash(item.content as ClipboardContent)
      this.repository.saveItem(item.content as ClipboardContent, hash)
    }
    return { success, message: success ? 'Restored as Text' : 'Failed' }
  }

  public async restoreClipboardFromDatabaseAsOcrText(itemId: string): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record || !record.content_ocr) return { success: false, message: 'No OCR text' }

    this.poller.setContents({
      availableFormats: ['text'],
      text: record.content_ocr
    })
    return { success: true, message: 'Restored OCR text' }
  }

  public async parseQRCodeFromItem(itemId: string): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record || !record.content_image) return { success: false, message: 'Not an image' }

    try {
      const text = await qrCodeService.parseFromBuffer(record.content_image)
      if (text) {
        this.poller.setContents({ availableFormats: ['text'], text })
        return { success: true, message: 'QR Code parsed' }
      }
      return { success: false, message: 'No QR Code found' }
    } catch (e) {
      return { success: false, message: 'Parse error' }
    }
  }

  public async parseOcrFromItem(itemId: string): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record || !record.content_image) return { success: false, message: 'Not an image' }

    try {
      // 使用repository的updateOCR方法来触发OCR解析
      // 这里直接调用ocrService进行OCR解析
      logger.info('[ClipboardManagerWorker] Parsing OCR from item:', itemId)
      if (!ocrService.ready) {
        const initialized = await this.ensureOCRServiceInitialized()
        if (!initialized) {
          return { success: false, message: 'OCR service initialization failed' }
        }
      }

      const ocrResult = await ocrService.recognizeText(record.content_image)
      if (ocrResult.success && ocrResult.text && ocrResult.text.trim()) {
        // 更新数据库中的OCR字段
        const updated = this.repository.updateOCR(itemId, ocrResult.text)
        if (updated) {
          // 发送事件通知前端更新
          this.sendEvent('record-ocr-updated', {
            contentId: itemId,
            ocrText: ocrResult.text,
            timestamp: Date.now()
          })
          return { success: true, message: 'OCR parsed successfully' }
        }
        return { success: false, message: 'Failed to update OCR' }
      }
      return { success: false, message: 'No text recognized' }
    } catch (e) {
      logger.error('[ClipboardManagerWorker] OCR parse error:', e)
      return { success: false, message: 'OCR parse error' }
    }
  }

  public setImageToClipboard(imageBuffer: any): boolean {
    // Logic from original worker
    // Need to handle buffer conversion?
    // Processor or Poller could handle this?
    // Poller setContents expects raw data.
    let buffer: Buffer
    if (Buffer.isBuffer(imageBuffer)) buffer = imageBuffer
    else if (imageBuffer instanceof Uint8Array) buffer = Buffer.from(imageBuffer)
    else if (imageBuffer?.data) buffer = Buffer.from(imageBuffer.data)
    else return false

    this.poller.setContents({
      availableFormats: ['image'],
      image: { data: buffer, size: buffer.length, width: 0, height: 0 }
    })
    return true
  }

  public handleMessage(message: WorkerMessage): void {
    if (message.type === 'request') {
      this.handleRequest(message as WorkerRequest)
    }
  }

  private getLastItemId(): string {
    // Helper to access repo state indirectly?
    // Actually handleClipboardChange uses result.id. 
    // This is not a critical method, just for logic check.
    return ''
  }

  private async handleRequest(request: WorkerRequest): Promise<void> {
    try {
      let result: any

      switch (request.method) {
        case 'initialize': result = await this.initialize(request.params); break
        case 'updateConfig': result = await this.updateConfig(request.params); break
        case 'getConfig': result = this.getConfig(); break
        case 'startListening': result = this.startListening(); break
        case 'stopListening': result = this.stopListening(); break
        case 'isListeningActive': result = this.isListeningActive(); break
        case 'syncClipboardHash': result = await this.syncClipboardHash(); break
        case 'resetErrorState': result = this.resetErrorState(); break
        case 'syncDeletedRecords': result = this.repository.syncDeletedRecords(request.params); break
        case 'syncDeletedRecordsByHash': result = this.repository.syncDeletedRecordsByHash(request.params); break
        case 'syncDeletedRecordsByIds': result = this.repository.syncDeletedRecordsByIds(request.params); break
        case 'exportHeapdump': result = this.exportHeapdump(request.params); break
        case 'restoreClipboardContent': result = this.restoreClipboardContent(request.params); break
        case 'restoreClipboardContentAsText': result = this.restoreClipboardContentAsText(request.params); break
        case 'restoreClipboardFromDatabase': result = await this.restoreClipboardFromDatabase(request.params); break
        case 'restoreClipboardFromDatabaseAsText': result = await this.restoreClipboardFromDatabaseAsText(request.params); break
        case 'restoreClipboardFromDatabaseAsOcrText': result = await this.restoreClipboardFromDatabaseAsOcrText(request.params); break
        case 'parseQRCodeFromItem': result = await this.parseQRCodeFromItem(request.params); break
        case 'parseOcrFromItem': result = await this.parseOcrFromItem(request.params); break
        case 'setImageToClipboard': result = this.setImageToClipboard(request.params); break
        case 'getCurrentClipboard': result = await this.getCurrentClipboard(); break
        case 'updateActiveWindow': result = this.updateActiveWindow(request.params); break
        default: throw new Error(`Unknown method: ${request.method}`)
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
  data: { workerName: '剪贴板管理器 Worker (Modular)' },
  timestamp: Date.now()
}
parentPort?.postMessage(readyEvent)
