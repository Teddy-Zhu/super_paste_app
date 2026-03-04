import { OCRManager, OCRManagerOptions, OCRConfig, OCROutput } from '../ocr/ocr-manager'
import logger from '../utils/logger'
import fs from 'fs'
import path from 'path'

export type OCRMode = 'local' | 'remote'
export type OCRRemoteRequestFormat = 'json' | 'form'

export interface OCRRemoteConfig {
  url?: string
  extraParams?: string
  imageFieldName?: string
  requestFormat?: OCRRemoteRequestFormat
  resultPath?: string
}

interface ParsedRemoteConfig {
  url: string
  extraParams: Record<string, unknown>
  imageFieldName: string
  requestFormat: OCRRemoteRequestFormat
  resultPath: string
}

/**
 * OCR 服务配置选项
 */
export interface OCRServiceOptions extends Partial<OCRManagerOptions> {
  mode?: OCRMode
  ocrConfig?: OCRConfig
  remoteConfig?: OCRRemoteConfig
}

export interface OCRRecognizeResult {
  success: boolean
  text: string
  confidence: number
  details?: OCROutput | Record<string, unknown>
  error?: string
}

/**
 * OCR 服务类
 * 为应用提供 OCR 识别功能的服务层
 */
export class OCRService {
  private static instance: OCRService | null = null
  private isServiceReady = false
  private ocrManager: OCRManager | null = null
  private mode: OCRMode = 'local'
  private remoteConfig: ParsedRemoteConfig | null = null

  private constructor() { }

  /**
   * 获取 OCR 服务单例
   */
  public static getInstance(): OCRService {
    if (!OCRService.instance) {
      OCRService.instance = new OCRService()
    }
    return OCRService.instance
  }

  /**
   * 初始化 OCR 服务
   * @param options 服务配置选项，包含资源路径和 OCR 配置
   */
  public async initialize(options: OCRServiceOptions): Promise<void> {
    try {
      logger.info('正在初始化 OCR 服务...')
      this.mode = options.mode || 'local'

      if (this.mode === 'remote') {
        this.setupRemoteConfig(options.remoteConfig)

        if (this.ocrManager) {
          this.ocrManager.destroy()
          this.ocrManager = null
        }

        this.isServiceReady = true
        logger.info('OCR 远程服务初始化成功')
        return
      }

      if (!options.resourcePath) {
        throw new Error('OCR 资源路径为空')
      }

      this.remoteConfig = null

      // 初始化 OCR 管理器
      this.ocrManager = OCRManager.getInstance({
        resourcePath: options.resourcePath,
        isDev: options.isDev
      })

      // 使用默认配置或用户提供的配置
      const defaultConfig: OCRConfig = {
        detRatio: 1.0,
        recImgHeight: 48,
        enableSpaceOptimize: true,
        enableDocCls: false
      }

      const finalConfig = { ...defaultConfig, ...options.ocrConfig }

      await this.ocrManager.initialize(finalConfig)
      this.isServiceReady = true

      logger.info('OCR 本地服务初始化成功')
    } catch (error) {
      this.isServiceReady = false
      logger.error('OCR 服务初始化失败:', error)
      throw error
    }
  }

  /**
   * 识别图片中的文字
   * @param imageData 图片数据（base64、文件路径或 Buffer）
   * @returns 识别结果
   */
  public async recognizeText(imageData: string | Buffer): Promise<OCRRecognizeResult> {
    if (!this.isServiceReady) {
      return {
        success: false,
        text: '',
        confidence: 0,
        error: 'OCR 服务未初始化'
      }
    }

    if (this.mode === 'remote') {
      return this.recognizeByRemote(imageData)
    }

    if (!this.ocrManager) {
      return {
        success: false,
        text: '',
        confidence: 0,
        error: '本地 OCR 管理器未初始化'
      }
    }

    try {
      const result = await this.ocrManager.recognize(imageData)
      const text = this.ocrManager.extractText(result)

      // 计算平均置信度
      const totalConfidence = result.src.reduce((sum, item) => sum + item.mean, 0)
      const confidence = result.src.length > 0 ? totalConfidence / result.src.length : 0

      return {
        success: true,
        text,
        confidence: Math.round(confidence * 100) / 100,
        details: result
      }
    } catch (error) {
      logger.error('OCR 识别失败:', error)
      return {
        success: false,
        text: '',
        confidence: 0,
        error: error instanceof Error ? error.message : String(error)
      }
    }
  }

  /**
   * 快速识别（仅返回文本，不返回详细信息）
   */
  public async quickRecognize(imageData: string | Buffer): Promise<string> {
    const result = await this.recognizeText(imageData)
    return result.success ? result.text : ''
  }

  /**
   * 检测图片中的文字区域（不识别内容）
   */
  public async detectTextRegions(imageData: string | Buffer): Promise<{
    success: boolean
    regions: any[]
    error?: string
  }> {
    if (this.mode === 'remote') {
      return {
        success: false,
        regions: [],
        error: '远程 OCR 模式暂不支持文字区域检测'
      }
    }

    if (!this.isServiceReady || !this.ocrManager) {
      return {
        success: false,
        regions: [],
        error: 'OCR 服务未初始化'
      }
    }

    try {
      const regions = await this.ocrManager.detect(imageData)
      return {
        success: true,
        regions
      }
    } catch (error) {
      logger.error('文字区域检测失败:', error)
      return {
        success: false,
        regions: [],
        error: error instanceof Error ? error.message : String(error)
      }
    }
  }

  /**
   * 批量识别多张图片
   */
  public async batchRecognize(imageDataList: (string | Buffer)[]): Promise<{
    success: boolean
    results: Array<{
      index: number
      text: string
      confidence: number
      success: boolean
      error?: string
    }>
    totalProcessed: number
    totalSuccess: number
  }> {
    if (!this.isServiceReady) {
      return {
        success: false,
        results: [],
        totalProcessed: 0,
        totalSuccess: 0
      }
    }

    const results: Array<{
      index: number
      text: string
      confidence: number
      success: boolean
      error?: string
    }> = []

    let totalSuccess = 0

    for (let i = 0; i < imageDataList.length; i++) {
      try {
        const result = await this.recognizeText(imageDataList[i])
        results.push({
          index: i,
          text: result.text,
          confidence: result.confidence,
          success: result.success,
          error: result.error
        })

        if (result.success) {
          totalSuccess++
        }
      } catch (error) {
        results.push({
          index: i,
          text: '',
          confidence: 0,
          success: false,
          error: error instanceof Error ? error.message : String(error)
        })
      }
    }

    return {
      success: totalSuccess > 0,
      results,
      totalProcessed: imageDataList.length,
      totalSuccess
    }
  }

  /**
   * 验证图片格式是否支持
   */
  public validateImageFormat(imageData: string): boolean {
    if (typeof imageData !== 'string') {
      return false
    }

    // 检查是否是 base64 格式
    if (imageData.startsWith('data:image/')) {
      const supportedFormats = ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp']
      return supportedFormats.some((format) => imageData.includes(`image/${format}`))
    }

    // 检查是否是文件路径
    if (imageData.includes('.')) {
      const ext = imageData.split('.').pop()?.toLowerCase()
      return ['png', 'jpg', 'jpeg', 'gif', 'bmp', 'webp'].includes(ext || '')
    }

    return false
  }

  /**
   * 获取 OCR 服务状态
   */
  public getServiceStatus(): {
    isReady: boolean
    resourcePath: string
    ocrInitialized: boolean
    mode: OCRMode
  } {
    return {
      isReady: this.isServiceReady,
      resourcePath: this.ocrManager?.resourcesPath || '',
      ocrInitialized: this.ocrManager?.initialized || false,
      mode: this.mode
    }
  }

  /**
   * 重新初始化 OCR 服务
   */
  public async reinitialize(options: OCRServiceOptions): Promise<void> {
    logger.info('重新初始化 OCR 服务...')

    // 销毁现有实例
    if (this.ocrManager) {
      this.ocrManager.destroy()
    }
    this.isServiceReady = false
    this.ocrManager = null

    // 重新初始化
    await this.initialize(options)
  }

  /**
   * 停止 OCR 服务
   */
  public stop(): void {
    if (this.isServiceReady && this.ocrManager) {
      this.ocrManager.destroy()
    }
    this.isServiceReady = false
    this.ocrManager = null
    this.remoteConfig = null
    logger.info('OCR 服务已停止')
  }

  /**
   * 检查服务是否就绪
   */
  public get ready(): boolean {
    return this.isServiceReady
  }

  private setupRemoteConfig(config?: OCRRemoteConfig): void {
    const url = config?.url?.trim() || ''
    if (!url) {
      throw new Error('远程 OCR 地址不能为空')
    }

    let extraParams: Record<string, unknown> = {}
    const extraParamsText = config?.extraParams?.trim()
    if (extraParamsText) {
      try {
        const parsed = JSON.parse(extraParamsText)
        if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
          extraParams = parsed as Record<string, unknown>
        } else {
          throw new Error('额外参数必须是 JSON 对象')
        }
      } catch (error) {
        throw new Error(
          `远程 OCR 额外参数 JSON 解析失败: ${error instanceof Error ? error.message : String(error)}`
        )
      }
    }

    this.remoteConfig = {
      url,
      extraParams,
      imageFieldName: config?.imageFieldName?.trim() || 'image',
      requestFormat: config?.requestFormat || 'json',
      resultPath: config?.resultPath?.trim() || 'data.result'
    }
  }

  private async recognizeByRemote(imageData: string | Buffer): Promise<OCRRecognizeResult> {
    if (!this.remoteConfig) {
      return {
        success: false,
        text: '',
        confidence: 0,
        error: '远程 OCR 配置未初始化'
      }
    }

    try {
      const { buffer, mimeType, fileName } = this.resolveImageData(imageData)
      const responseData = await this.requestRemoteOCR(buffer, mimeType, fileName)
      const extracted = this.readResultByPath(responseData, this.remoteConfig.resultPath)

      if (typeof extracted === 'undefined' || extracted === null) {
        return {
          success: false,
          text: '',
          confidence: 0,
          details: responseData,
          error: `未找到返回字段: ${this.remoteConfig.resultPath}`
        }
      }

      const text = this.normalizeOCRText(extracted)
      if (!text.trim()) {
        return {
          success: false,
          text: '',
          confidence: 0,
          details: responseData,
          error: '远程 OCR 返回内容为空'
        }
      }

      return {
        success: true,
        text,
        confidence: 1,
        details: responseData
      }
    } catch (error) {
      logger.error('远程 OCR 识别失败:', error)
      return {
        success: false,
        text: '',
        confidence: 0,
        error: error instanceof Error ? error.message : String(error)
      }
    }
  }

  private resolveImageData(imageData: string | Buffer): {
    buffer: Buffer
    mimeType: string
    fileName: string
  } {
    if (Buffer.isBuffer(imageData)) {
      return {
        buffer: imageData,
        mimeType: 'image/png',
        fileName: 'ocr-image.png'
      }
    }

    if (typeof imageData !== 'string' || !imageData.trim()) {
      throw new Error('无效的图片数据')
    }

    const trimmed = imageData.trim()

    const dataUriMatch = trimmed.match(/^data:(image\/[a-zA-Z0-9.+-]+);base64,(.+)$/)
    if (dataUriMatch) {
      return {
        buffer: Buffer.from(dataUriMatch[2], 'base64'),
        mimeType: dataUriMatch[1],
        fileName: this.getDefaultFileName(dataUriMatch[1])
      }
    }

    if (fs.existsSync(trimmed)) {
      const buffer = fs.readFileSync(trimmed)
      const ext = path.extname(trimmed).replace('.', '').toLowerCase()
      return {
        buffer,
        mimeType: this.resolveMimeType(ext),
        fileName: path.basename(trimmed)
      }
    }

    return {
      buffer: Buffer.from(trimmed, 'base64'),
      mimeType: 'image/png',
      fileName: 'ocr-image.png'
    }
  }

  private async requestRemoteOCR(
    imageBuffer: Buffer,
    mimeType: string,
    fileName: string
  ): Promise<Record<string, unknown>> {
    if (!this.remoteConfig) {
      throw new Error('远程 OCR 配置未初始化')
    }

    const { url, extraParams, imageFieldName, requestFormat } = this.remoteConfig
    let response: Response

    if (requestFormat === 'form') {
      const formData = new FormData()
      for (const [key, value] of Object.entries(extraParams)) {
        formData.append(key, this.stringifyParamValue(value))
      }
      const blob = new Blob([Uint8Array.from(imageBuffer)], { type: mimeType })
      formData.append(imageFieldName, blob, fileName)

      response = await fetch(url, {
        method: 'POST',
        body: formData
      })
    } else {
      const payload: Record<string, unknown> = {
        ...extraParams,
        [imageFieldName]: imageBuffer.toString('base64')
      }

      response = await fetch(url, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify(payload)
      })
    }

    const text = await response.text()
    let data: Record<string, unknown>
    try {
      data = text ? (JSON.parse(text) as Record<string, unknown>) : {}
    } catch {
      throw new Error(`远程 OCR 响应不是合法 JSON，HTTP 状态: ${response.status}`)
    }

    if (!response.ok) {
      const errorMessage =
        (typeof data?.message === 'string' && data.message) ||
        (typeof data?.error === 'string' && data.error) ||
        `远程 OCR 请求失败，HTTP 状态: ${response.status}`
      throw new Error(errorMessage)
    }

    return data
  }

  private readResultByPath(data: Record<string, unknown>, resultPath: string): unknown {
    if (!resultPath) {
      return data
    }

    const pathParts = resultPath
      .replace(/\[(\d+)\]/g, '.$1')
      .split('.')
      .map((item) => item.trim())
      .filter(Boolean)

    let current: unknown = data
    for (const key of pathParts) {
      if (Array.isArray(current)) {
        const index = Number(key)
        current = Number.isNaN(index) ? undefined : current[index]
        continue
      }

      if (current && typeof current === 'object') {
        current = (current as Record<string, unknown>)[key]
        continue
      }

      return undefined
    }

    return current
  }

  private normalizeOCRText(value: unknown): string {
    if (typeof value === 'string') {
      return value
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
      return String(value)
    }
    if (Array.isArray(value)) {
      return value.map((item) => this.normalizeOCRText(item)).join('\n')
    }
    if (value && typeof value === 'object') {
      return JSON.stringify(value)
    }
    return ''
  }

  private stringifyParamValue(value: unknown): string {
    if (typeof value === 'string') {
      return value
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
      return String(value)
    }
    if (value === null || typeof value === 'undefined') {
      return ''
    }
    return JSON.stringify(value)
  }

  private getDefaultFileName(mimeType: string): string {
    const ext = mimeType.split('/')[1] || 'png'
    return `ocr-image.${ext}`
  }

  private resolveMimeType(ext: string): string {
    const mimeMap: Record<string, string> = {
      png: 'image/png',
      jpg: 'image/jpeg',
      jpeg: 'image/jpeg',
      gif: 'image/gif',
      bmp: 'image/bmp',
      webp: 'image/webp'
    }
    return mimeMap[ext] || 'image/png'
  }
}

// 注意：现在需要在使用前调用 ocrService.initialize(options) 来初始化
// 导出服务实例
export const ocrService = OCRService.getInstance()
