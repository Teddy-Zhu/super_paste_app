import { app, nativeImage } from 'electron'
import path from 'node:path'
import clipboardManager from '../features/clipboard/manager/clipboard-manager'
import clipboardDatabase from '../infrastructure/database/clipboard-database'
import clipboardHistoryService from '../services/clipboard-history-service'
import {
  ClipboardDataConverter,
  type ClipboardContent,
  type ClipboardContentForUI,
  type ClipboardManagerConfig
} from '../features/clipboard/types/clipboard-types'
import { OCRWorkerClient } from '../ocr/worker-client'
import { getInstalledOCRModel } from '../ocr/model-store'
import { recordAIOCRUsage } from '../ocr/usage-service'
import { isAIOCRProvider } from '../../shared/ai-ocr'
import {
  MCP_MAX_IMAGE_BYTES,
  type ClipboardChangeData,
  type MCPClipboardChange
} from '../../shared/mcp'
import type { OCRServiceOptions } from '../services/ocr-service'
import type { MCPCapabilities, MCPClipboardResult } from './tools'
import { ClipboardChanges } from './clipboard-changes'
import ocrWorkerPath from '../ocr/ocr.worker?modulePath'

function requireClipboard(): void {
  if (!clipboardManager.isRunning()) throw new Error('剪贴板服务尚未就绪，请检查系统访问权限')
}

function serializeClipboard(
  content: ClipboardContent | ClipboardContentForUI,
  includeImage: boolean
): MCPClipboardResult {
  const { image, ...textContent } = content
  const output: MCPClipboardResult = { data: { ...textContent } }
  if (image?.length) {
    const buffer = Buffer.from(image)
    output.data.image = { bytes: buffer.length }
    if (includeImage) {
      const decoded = validateImage(buffer)
      const png = decoded.toPNG()
      if (png.length > MCP_MAX_IMAGE_BYTES)
        throw new Error('PNG 图片超过 10 MiB，请关闭 includeImage')
      output.data.image = { bytes: png.length, mimeType: 'image/png', ...decoded.getSize() }
      output.image = { data: png.toString('base64'), mimeType: 'image/png' }
    }
  }
  return output
}

function validateImage(buffer: Buffer): Electron.NativeImage {
  if (!buffer.length || buffer.length > MCP_MAX_IMAGE_BYTES)
    throw new Error('图片须为 1 字节至 10 MiB')
  const image = nativeImage.createFromBuffer(buffer)
  if (image.isEmpty()) throw new Error('图片格式无效或不受支持')
  const { width, height } = image.getSize()
  if (width * height > 40000000) throw new Error('图片超过 4000 万像素，请缩小后重试')
  return image
}

function getHistoryItem(itemId: string): ReturnType<typeof ClipboardDataConverter.recordToItem> {
  const record = clipboardDatabase.getRecordByContentId(itemId)
  if (!record) throw new Error('剪贴板历史记录不存在')
  return ClipboardDataConverter.recordToItem(record)
}

export class AppMCPCapabilities implements MCPCapabilities {
  private readonly changes = new ClipboardChanges()
  private readonly ocr = new OCRWorkerClient(ocrWorkerPath)
  private recognizing = false
  private stopped = false
  private ocrStopping: Promise<void> | undefined
  private readonly onChange = (change: ClipboardChangeData): void => {
    this.changes.record(change)
  }

  constructor() {
    clipboardManager.on('clipboard-content-changed', this.onChange)
  }

  public async getClipboard(includeImage: boolean): Promise<MCPClipboardResult> {
    requireClipboard()
    // 先取游标，再读取内容；并发复制最多导致重复读取，不会跳过变化。
    const cursor = this.changes.cursor
    const content = await clipboardManager.call<ClipboardContent>('getCurrentClipboard')
    const output = serializeClipboard(content, includeImage)
    output.data.cursor = cursor
    return output
  }

  public async waitForChange(
    cursor: string | undefined,
    timeoutMs: number,
    signal: AbortSignal
  ): Promise<Record<string, unknown>> {
    requireClipboard()
    const initial = cursor ?? this.changes.cursor
    if (!(await clipboardManager.call<boolean>('isListeningActive')))
      throw new Error('剪贴板监听未开启，请先在调试设置中启用监听')
    return this.changes.wait(initial, timeoutMs, signal)
  }

  public async subscribeChanges(
    listener: (change: MCPClipboardChange) => void
  ): Promise<() => void> {
    requireClipboard()
    if (!(await clipboardManager.call<boolean>('isListeningActive')))
      throw new Error('剪贴板监听未开启，请先在调试设置中启用监听')
    if (this.stopped) throw new Error('MCP 服务已停止')
    return this.changes.subscribe(listener)
  }

  public async searchHistory(options: {
    searchText?: string
    limit: number
    offset: number
  }): Promise<Record<string, unknown>> {
    const { items } = await clipboardHistoryService.searchHistory({
      ...options,
      orderBy: 'timestamp'
    })
    return {
      items: items.map(({ content, ...item }) => ({
        ...item,
        content: serializeClipboard(content, false).data
      })),
      limit: options.limit,
      offset: options.offset
    }
  }

  public async getHistoryItem(itemId: string, includeImage: boolean): Promise<MCPClipboardResult> {
    const { content, ...item } = getHistoryItem(itemId)
    const output = serializeClipboard(content, includeImage)
    output.data = { ...item, content: output.data }
    return output
  }

  public async setText(text: string): Promise<void> {
    requireClipboard()
    if (!(await clipboardManager.call<boolean>('restoreClipboardContent', { content: { text } })))
      throw new Error('写入系统剪贴板失败')
  }

  public async recognizeImage(input: {
    imageBase64?: string
    itemId?: string
  }): Promise<Record<string, unknown>> {
    if (this.stopped) throw new Error('MCP 服务已停止')
    if (this.recognizing) throw new Error('MCP OCR 正在识别，请稍后重试')
    this.recognizing = true
    try {
      requireClipboard()
      let buffer: Buffer
      if (input.imageBase64) {
        const base64 = input.imageBase64
          .replace(/^data:image\/[a-zA-Z0-9.+-]+;base64,/, '')
          .replace(/\s/g, '')
        if (!/^[A-Za-z0-9+/]+={0,2}$/.test(base64)) throw new Error('图片 Base64 无效')
        buffer = Buffer.from(base64, 'base64')
        if (buffer.toString('base64').replace(/=+$/, '') !== base64.replace(/=+$/, ''))
          throw new Error('图片 Base64 无效')
      } else {
        const content = input.itemId
          ? getHistoryItem(input.itemId).content
          : await clipboardManager.call<ClipboardContent>('getCurrentClipboard')
        if (!content.image?.length) throw new Error('剪贴板内容不包含图片')
        buffer = Buffer.from(content.image)
      }
      validateImage(buffer)
      const config = await clipboardManager.call<ClipboardManagerConfig>('getConfig')
      let options: OCRServiceOptions
      if (config.ocrMode === 'local') {
        const resourcePath = path.join(app.getPath('userData'), 'ocr-models')
        if (!(await getInstalledOCRModel(resourcePath, config.ocrLocalModel, false)))
          throw new Error('本地 OCR 模型未下载，请先在 OCR 设置中下载当前模型')
        options = { mode: 'local', resourcePath, modelId: config.ocrLocalModel }
      } else if (isAIOCRProvider(config.ocrMode)) {
        options = {
          mode: config.ocrMode,
          aiConfig: config.ocrAIConfigs[config.ocrMode],
          onAIUsage: recordAIOCRUsage
        }
      } else {
        options = { mode: 'remote', remoteConfig: config.ocrRemoteConfig }
      }
      if (this.stopped) throw new Error('MCP 服务已停止')
      await this.ocr.initialize(options)
      const result = await this.ocr.recognizeText(buffer)
      if (!result.success) throw new Error(result.error || 'OCR 识别失败')
      return { text: result.text, confidence: result.confidence, mode: config.ocrMode }
    } finally {
      try {
        await this.stopOCR()
      } finally {
        this.recognizing = false
      }
    }
  }

  public async stop(): Promise<void> {
    this.stopped = true
    clipboardManager.off('clipboard-content-changed', this.onChange)
    await this.stopOCR()
  }

  private stopOCR(): Promise<void> {
    return (this.ocrStopping ??= this.ocr.stop().finally(() => {
      this.ocrStopping = undefined
    }))
  }
}
