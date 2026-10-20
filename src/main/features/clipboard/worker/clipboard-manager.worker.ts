import { DEFAULT_OCR_MODEL_ID, getOCRModel } from '../../../../shared/ocr-models'
import { createHash } from 'node:crypto'
import { OCRTaskQueue, type OCRJob } from '../../../ocr/task-queue'
import { createDefaultAIOCRConfigs, isAIOCRProvider } from '../../../../shared/ai-ocr'
import { parentPort, workerData } from 'node:worker_threads'
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
import { OCRWorkerClient } from '../../../ocr/worker-client'
import type { OCRServiceOptions } from '../../../services/ocr-service'
import type { ActiveWindowInfo } from '../../../services/active-window-provider'
// Modules
import { ClipboardPoller } from './modules/clipboard-poller'
import { ContentProcessor } from './modules/content-processor'
import { ClipboardRepository } from './modules/clipboard-repository'
import { ClipboardSourceTracker } from './modules/clipboard-source-tracker'

const ocrService = new OCRWorkerClient(workerData.ocrWorkerPath)

class ClipboardManagerWorker extends BaseWorker {
  private config: ClipboardManagerConfig

  // Modules
  private poller: ClipboardPoller
  private processor: ContentProcessor
  private repository: ClipboardRepository

  // State
  private activeWindowInitialized: boolean = false
  private sourceTracker = new ClipboardSourceTracker()
  private appSourceIssue: string | undefined
  private ocrResourcePath: string = ''
  private ocrUpdateQueue: Promise<void> = Promise.resolve()
  private ocrInitializationError = ''
  private ocrInitializing: Promise<boolean> | null = null
  private ocrConfigHash: string = ''
  private ocrQueue: OCRTaskQueue
  private suspendedModels = new Set<string>()

  constructor() {
    super()
    this.config = {
      maxHistorySize: 100,
      enableAutoSave: false,
      enableLocalOCR: false,
      ocrMode: 'local',
      ocrLocalModel: DEFAULT_OCR_MODEL_ID,
      ocrAIConfigs: createDefaultAIOCRConfigs(),
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
    this.ocrQueue = new OCRTaskQueue(
      (job) => this.runOCRJob(job),
      (status) => {
        if (status.state === 'error') logger.error('[OCR] 后台任务失败:', status.error)
        this.sendEvent('ocr:task-status', status)
      }
    )
  }

  protected getWorkerName(): string {
    return '剪贴板变化检测 Worker'
  }

  public async initialize(
    params?:
      | Partial<ClipboardManagerConfig>
      | {
          config?: Partial<ClipboardManagerConfig>
          userDataPath: string
        }
  ): Promise<void> {
    // 处理参数兼容性
    let config: Partial<ClipboardManagerConfig> | undefined
    let userDataPath: string | undefined
    if (params && 'config' in params) {
      config = params.config
      userDataPath = params.userDataPath
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
    if (userDataPath) this.ocrResourcePath = path.join(userDataPath, 'ocr-models')

    // 初始化时进行一次全量检查
    try {
      const rawData = this.poller.readClipboardDirect()
      const converted = rawData
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

  private buildOCRServiceOptions(): OCRServiceOptions | null {
    if (this.config.ocrMode === 'local') {
      if (!this.ocrResourcePath) {
        this.ocrInitializationError = 'OCR 模型缓存路径尚未初始化'
        logger.warn('[剪贴板管理器 Worker]', this.ocrInitializationError)
        return null
      }

      return {
        mode: 'local',
        resourcePath: this.ocrResourcePath,
        modelId: this.config.ocrLocalModel
      }
    }

    if (isAIOCRProvider(this.config.ocrMode)) {
      return {
        mode: this.config.ocrMode,
        aiConfig: this.config.ocrAIConfigs[this.config.ocrMode],
        onAIUsage: (entry) => this.sendEvent('ai-ocr-usage', entry)
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
      modelId: this.config.ocrLocalModel,
      remoteConfig: this.config.ocrRemoteConfig,
      aiConfigs: this.config.ocrAIConfigs
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
      await this.ocrInitializing
      return this.ensureOCRServiceInitialized(forceReinitialize)
    }

    this.ocrInitializing = (async () => {
      try {
        if (ocrService.ready) {
          await ocrService.reinitialize(options)
        } else {
          await ocrService.initialize(options)
        }
        this.ocrInitializationError = ''
        this.ocrConfigHash = nextHash
        logger.info('[剪贴板管理器 Worker] OCR 服务初始化完成')
        return true
      } catch (error) {
        this.ocrInitializationError = error instanceof Error ? error.message : String(error)
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
      if (this.isInErrorBackoff()) return

      const copiedAt = Date.now()
      const converted = this.processor.convertClipboardData(data)

      // Quick check
      const hasQuickChange = this.processor.checkQuickClipboardChange(converted)
      if (!hasQuickChange) {
        this.onOperationSuccess()
        return
      }

      // Freeze the source before asynchronous conversion can observe a later focus change.
      const source = this.sourceTracker.select(Boolean(converted.hasImage))
      const appInfo = this.config.enableAutoSave ? this.getCurrentAppInfo(source.window) : undefined
      if (this.config.enableAutoSave && source.reason !== 'active-window') {
        logger.debug('[ActiveWindowService] Clipboard source selected', {
          reason: source.reason,
          foregroundApplication: this.sourceTracker.activeWindow?.application,
          application: appInfo?.application,
          hasIcon: Boolean(appInfo?.icon)
        })
      }

      // Full process
      const content = await this.processor.buildClipboardContent(converted)
      const statisticsType = ClipboardDataConverter.detectPrimaryType(content)
      if (statisticsType !== 'empty') {
        this.sendEvent('copy-statistics', {
          timestamp: copiedAt,
          type: statisticsType,
          source: (source.window?.application ?? '').trim().slice(0, 200),
          bytes:
            (content.image?.byteLength ?? 0) +
            [content.text, content.html, content.rtf, ...(content.files ?? [])].reduce<number>(
              (size, text) => size + (text ? Buffer.byteLength(text, 'utf8') : 0),
              0
            ),
          files: content.files?.length ?? 0
        })
      }
      const hash = ClipboardDataConverter.generateContentHash(content) // or processor.generateClipboardHash(converted)?
      // processor.generateClipboardHash does buildClipboardContent then hash.
      // but we need 'content' to save it.

      // Check if repo has it (optimization to avoid building content if we could allow repo to just check hash?)
      // But repo needs content to insert if new.

      if (this.config.enableAutoSave) {
        const result = await this.repository.saveItem(content, hash, {
          appInfo,
          refreshSourceApp: source.reason === 'before-screenshot',
          enableLocalOCR: this.config.enableLocalOCR,
          onOCRRequested: (contentId) => {
            const result = this.requestOCR(contentId, true)
            if (!result.success) logger.error('[OCR] 自动识别排队失败:', result.message)
          }
        })

        if (result.isNew || result.sourceUpdated || result.id !== this.getLastItemId()) {
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
  public updateActiveWindow(windowInfo: ActiveWindowInfo | null): void {
    const previous = this.sourceTracker.activeWindow
    if (
      !this.activeWindowInitialized ||
      previous?.pid !== windowInfo?.pid ||
      previous?.application !== windowInfo?.application ||
      previous?.path !== windowInfo?.path
    ) {
      logger.info('[ActiveWindowService] Worker snapshot', {
        hasWindow: Boolean(windowInfo),
        application: windowInfo?.application,
        pid: windowInfo?.pid,
        hasPath: Boolean(windowInfo?.path),
        hasIcon: Boolean(windowInfo?.icon)
      })
    }
    this.sourceTracker.update(windowInfo)
    this.activeWindowInitialized = true
  }

  private getCurrentAppInfo(activeWindow: ActiveWindowInfo | null): AppInfo | undefined {
    try {
      const issue = !this.activeWindowInitialized
        ? 'not-initialized'
        : !activeWindow
          ? 'no-active-window'
          : !activeWindow.application
            ? 'missing-application'
            : !activeWindow.path
              ? 'missing-executable-path'
              : undefined
      if (!activeWindow?.application || !activeWindow.path) {
        if (issue !== this.appSourceIssue) {
          logger.warn('[ActiveWindowService] Clipboard source unavailable', {
            reason: issue,
            application: activeWindow?.application,
            pid: activeWindow?.pid
          })
          this.appSourceIssue = issue
        }
        return undefined
      }
      if (this.appSourceIssue) {
        logger.info('[ActiveWindowService] Clipboard source recovered')
        this.appSourceIssue = undefined
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
      logger.warn('[ActiveWindowService] Clipboard source conversion failed:', error)
      return undefined
    }
  }

  // Delegate Methods
  public getConfig(): ClipboardManagerConfig {
    return { ...this.config }
  }

  public updateConfig(newConfig: Partial<ClipboardManagerConfig>): Promise<void> {
    const previousHash = this.buildOCRConfigHash()
    const previousAuto = this.config.enableLocalOCR
    this.config = {
      ...this.config,
      ...newConfig,
      ocrRemoteConfig: { ...this.config.ocrRemoteConfig, ...newConfig.ocrRemoteConfig }
    }
    if (!this.config.enableLocalOCR) this.ocrQueue.cancelAutomatic()
    if (previousHash !== this.buildOCRConfigHash() || previousAuto !== this.config.enableLocalOCR) {
      const idle = this.ocrQueue.pause()
      this.ocrUpdateQueue = this.ocrUpdateQueue
        .then(async () => {
          await idle
          await ocrService.stop()
          this.ocrConfigHash = ''
        })
        .catch((error) => {
          logger.error('[OCR] 更新后台服务失败:', error)
          this.ocrQueue.failPending(error instanceof Error ? error.message : String(error))
        })
    }
    return Promise.resolve()
  }

  public isListeningActive(): boolean {
    return this.poller.isListeningActive()
  }

  public async syncClipboardHash(): Promise<void> {
    try {
      const raw = this.poller.readClipboardDirect()
      const converted = raw
      await this.processor.generateClipboardHash(converted)
      // We might want to update repo's 'lastItemHash' without saving?
      // Or just update internal state (which is now in repo).
      // If we want to align, we might need a method in repo `setLastItemHash`?
      // Or just let it be.
      this.processor.checkQuickClipboardChange(converted)
    } catch (error) {
      logger.warn('[ClipboardManagerWorker] 同步剪贴板哈希失败:', error)
    }
    this.resetErrorState()
  }

  public async getCurrentClipboard(): Promise<ClipboardContent> {
    const raw = this.poller.readClipboardDirect()
    return this.processor.buildClipboardContent(raw)
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
    } catch (error) {
      logger.error('[ClipboardManagerWorker] 还原纯文本失败:', error)
      return false
    }
  }

  public async restoreClipboardFromDatabase(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
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

  public async restoreClipboardFromDatabaseAsText(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record) return { success: false, message: 'Item not found' }

    const item = ClipboardDataConverter.recordToItem(record)
    const success = this.restoreClipboardContentAsText({
      content: item.content as ClipboardContent
    })

    if (success) {
      const hash = ClipboardDataConverter.generateContentHash(item.content as ClipboardContent)
      this.repository.saveItem(item.content as ClipboardContent, hash)
    }
    return { success, message: success ? 'Restored as Text' : 'Failed' }
  }

  public async restoreClipboardFromDatabaseAsOcrText(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record || !record.content_ocr) return { success: false, message: 'No OCR text' }

    this.poller.setContents({
      availableFormats: ['text'],
      text: record.content_ocr
    })
    return { success: true, message: 'Restored OCR text' }
  }

  public async parseQRCodeFromItem(
    itemId: string
  ): Promise<{ success: boolean; message?: string }> {
    const record = this.repository.getById(itemId)
    if (!record || !record.content_image) return { success: false, message: 'Not an image' }

    try {
      const text = await qrCodeService.parseFromBuffer(record.content_image)
      if (text) {
        this.poller.setContents({ availableFormats: ['text'], text })
        return { success: true, message: 'QR Code parsed' }
      }
      return { success: false, message: 'No QR Code found' }
    } catch (error) {
      logger.error('[ClipboardManagerWorker] 解析二维码失败:', error)
      return { success: false, message: 'Parse error' }
    }
  }

  public parseOcrFromItem(itemId: string): { success: boolean; message?: string } {
    return this.requestOCR(itemId, false)
  }

  private requestOCR(
    contentId: string,
    automatic: boolean
  ): { success: boolean; message?: string } {
    const record = this.repository.getById(contentId)
    if (!record?.content_image) return { success: false, message: '剪贴板记录不包含图片' }
    if (this.config.ocrMode === 'local' && this.suspendedModels.has(this.config.ocrLocalModel)) {
      this.sendEvent('ocr:task-status', {
        contentId,
        state: 'error',
        error: '本地模型正在维护，请稍后重试'
      })
      return { success: false, message: '本地模型正在维护，请稍后重试' }
    }
    try {
      const hash = createHash('sha256').update(record.content_image).digest('hex')
      const duplicate = this.ocrQueue.enqueue(hash, contentId, automatic)
      this.sendEvent('ocr-requested', undefined)
      return { success: true, message: duplicate ? 'OCR 已在队列中' : 'OCR 已加入后台队列' }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.sendEvent('ocr:task-status', { contentId, state: 'error', error: message })
      return { success: false, message }
    }
  }

  private async runOCRJob(job: OCRJob): Promise<void> {
    const record = [...job.records.keys()]
      .map((id) => this.repository.getById(id))
      .find((item) => item?.content_image)
    if (!record?.content_image) return
    if (!(await this.ensureOCRServiceInitialized()))
      throw new Error(this.ocrInitializationError || 'OCR 服务初始化失败')
    const result = await ocrService.recognizeText(record.content_image)
    if (!result.success) throw new Error(result.error || 'OCR 识别失败')
    if (!result.text.trim()) throw new Error('未识别到文字')
    for (const id of job.records.keys()) {
      if (!this.repository.getById(id)) continue
      if (!this.repository.updateOCR(id, result.text)) throw new Error('保存 OCR 识别结果失败')
      this.sendEvent('record-ocr-updated', {
        contentId: id,
        ocrText: result.text,
        timestamp: Date.now()
      })
    }
  }

  public async resumeOCR(): Promise<void> {
    while (true) {
      const update = this.ocrUpdateQueue
      await update
      if (update !== this.ocrUpdateQueue) continue
      if (!this.config.enableLocalOCR && !this.ocrQueue.hasJobs) return
      if (this.config.ocrMode === 'local' && this.suspendedModels.has(this.config.ocrLocalModel))
        return
      const hash = this.buildOCRConfigHash()
      const initialized = await this.ensureOCRServiceInitialized()
      if (update !== this.ocrUpdateQueue || hash !== this.buildOCRConfigHash()) continue
      if (initialized) this.ocrQueue.resume()
      else this.ocrQueue.failPending(this.ocrInitializationError || 'OCR 服务初始化失败')
      return
    }
  }

  public async suspendOCRModel(id: unknown): Promise<void> {
    const model = getOCRModel(id)
    this.suspendedModels.add(model.id)
    if (this.config.ocrMode === 'local' && this.config.ocrLocalModel === model.id) {
      await this.ocrQueue.pause()
      await this.ocrUpdateQueue
      await ocrService.stop()
      this.ocrConfigHash = ''
    }
  }

  public releaseOCRModel(id: unknown): void {
    this.suspendedModels.delete(getOCRModel(id).id)
  }

  public setImageToClipboard(imageBuffer: unknown): boolean {
    // Logic from original worker
    // Need to handle buffer conversion?
    // Processor or Poller could handle this?
    // Poller setContents expects raw data.
    let buffer: Buffer
    if (Buffer.isBuffer(imageBuffer)) buffer = imageBuffer
    else if (imageBuffer instanceof Uint8Array) buffer = Buffer.from(imageBuffer)
    else if (
      imageBuffer &&
      typeof imageBuffer === 'object' &&
      'data' in imageBuffer &&
      Array.isArray(imageBuffer.data) &&
      imageBuffer.data.every((value) => Number.isInteger(value) && value >= 0 && value <= 255)
    )
      buffer = Buffer.from(imageBuffer.data)
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
      let result: unknown

      switch (request.method) {
        case 'initialize':
          result = await this.initialize(
            request.params as Parameters<ClipboardManagerWorker['initialize']>[0]
          )
          break
        case 'updateConfig':
          result = await this.updateConfig(
            request.params as Parameters<ClipboardManagerWorker['updateConfig']>[0]
          )
          break
        case 'getConfig':
          result = this.getConfig()
          break
        case 'hasOCRJobs':
          result = this.ocrQueue.hasJobs
          break
        case 'resumeOCR':
          result = await this.resumeOCR()
          break
        case 'failPendingOCR':
          this.ocrQueue.failPending(String(request.params))
          break
        case 'suspendOCRModel':
          result = await this.suspendOCRModel(request.params)
          break
        case 'releaseOCRModel':
          result = this.releaseOCRModel(request.params)
          break
        case 'startListening':
          result = this.startListening()
          break
        case 'stopListening':
          result = this.stopListening()
          break
        case 'getAccessStatus':
          result = this.poller.getAccessStatus()
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
          result = this.repository.syncDeletedRecords(
            request.params as Parameters<ClipboardRepository['syncDeletedRecords']>[0]
          )
          break
        case 'syncDeletedRecordsByHash':
          result = this.repository.syncDeletedRecordsByHash(
            request.params as Parameters<ClipboardRepository['syncDeletedRecordsByHash']>[0]
          )
          break
        case 'syncDeletedRecordsByIds':
          result = this.repository.syncDeletedRecordsByIds(
            request.params as Parameters<ClipboardRepository['syncDeletedRecordsByIds']>[0]
          )
          break
        case 'exportHeapdump':
          result = this.exportHeapdump(
            request.params as Parameters<ClipboardManagerWorker['exportHeapdump']>[0]
          )
          break
        case 'restoreClipboardContent':
          result = this.restoreClipboardContent(
            request.params as Parameters<ClipboardManagerWorker['restoreClipboardContent']>[0]
          )
          break
        case 'restoreClipboardContentAsText':
          result = this.restoreClipboardContentAsText(
            request.params as Parameters<ClipboardManagerWorker['restoreClipboardContentAsText']>[0]
          )
          break
        case 'restoreClipboardFromDatabase':
          result = await this.restoreClipboardFromDatabase(
            request.params as Parameters<ClipboardManagerWorker['restoreClipboardFromDatabase']>[0]
          )
          break
        case 'restoreClipboardFromDatabaseAsText':
          result = await this.restoreClipboardFromDatabaseAsText(
            request.params as Parameters<
              ClipboardManagerWorker['restoreClipboardFromDatabaseAsText']
            >[0]
          )
          break
        case 'restoreClipboardFromDatabaseAsOcrText':
          result = await this.restoreClipboardFromDatabaseAsOcrText(
            request.params as Parameters<
              ClipboardManagerWorker['restoreClipboardFromDatabaseAsOcrText']
            >[0]
          )
          break
        case 'parseQRCodeFromItem':
          result = await this.parseQRCodeFromItem(
            request.params as Parameters<ClipboardManagerWorker['parseQRCodeFromItem']>[0]
          )
          break
        case 'parseOcrFromItem':
          result = await this.parseOcrFromItem(
            request.params as Parameters<ClipboardManagerWorker['parseOcrFromItem']>[0]
          )
          break
        case 'setImageToClipboard':
          result = this.setImageToClipboard(
            request.params as Parameters<ClipboardManagerWorker['setImageToClipboard']>[0]
          )
          break
        case 'getCurrentClipboard':
          result = await this.getCurrentClipboard()
          break
        case 'updateActiveWindow':
          result = this.updateActiveWindow(
            request.params as Parameters<ClipboardManagerWorker['updateActiveWindow']>[0]
          )
          break
        default:
          throw new Error(`Unknown method: ${request.method}`)
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
