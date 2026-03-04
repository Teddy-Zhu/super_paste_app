import { OCRManager, OCRManagerOptions, OCRConfig, OCROutput } from '../ocr/ocr-manager'
import logger from '../utils/logger'

/**
 * OCR 服务配置选项
 */
export interface OCRServiceOptions extends OCRManagerOptions {
  ocrConfig?: OCRConfig
}

/**
 * OCR 服务类
 * 为应用提供 OCR 识别功能的服务层
 */
export class OCRService {
  private static instance: OCRService | null = null
  private isServiceReady = false
  private ocrManager: OCRManager | null = null

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
      if (!options.resourcePath) {
        throw new Error('OCR 资源路径为空')
      }

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

      logger.info('OCR 服务初始化成功')
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
  public async recognizeText(imageData: string | Buffer): Promise<{
    success: boolean
    text: string
    confidence: number
    details?: OCROutput
    error?: string
  }> {
    if (!this.isServiceReady || !this.ocrManager) {
      return {
        success: false,
        text: '',
        confidence: 0,
        error: 'OCR 服务未初始化'
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
  } {
    return {
      isReady: this.isServiceReady,
      resourcePath: this.ocrManager?.resourcesPath || '',
      ocrInitialized: this.ocrManager?.initialized || false
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
      this.isServiceReady = false
      this.ocrManager = null
      logger.info('OCR 服务已停止')
    }
  }

  /**
   * 检查服务是否就绪
   */
  public get ready(): boolean {
    return this.isServiceReady
  }
}

// 注意：现在需要在使用前调用 ocrService.initialize(options) 来初始化
// 导出服务实例
export const ocrService = OCRService.getInstance()
