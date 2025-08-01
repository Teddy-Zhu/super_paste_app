import { setOCREnv, init } from 'esearch-ocr'
import * as ort from 'onnxruntime-node'
import { promises as fs } from 'fs'
import path from 'path'
import logger from './logger'
import { createCanvas, loadImage, createImageData } from 'canvas'

/**
 * OCR 识别结果类型
 */
export interface OCRResult {
  text: string
  mean: number
  box: [number, number][] // 文字框坐标 ↖ ↗ ↘ ↙
  style: {
    bg: [number, number, number] // RGB背景色
    text: [number, number, number] // RGB文字色
  }
}

/**
 * OCR 输出结果类型
 */
export interface OCROutput {
  src: OCRResult[] // 每个视觉行
  columns: {
    src: OCRResult[]
    outerBox: [number, number][]
    parragraphs: {
      src: OCRResult[]
      parse: OCRResult
    }[]
  }[]
  parragraphs: OCRResult[] // 聚合的段落
  readingDir: {
    inline: 'lr' | 'rl' | 'tb' | 'bt' // 行内阅读方向
    block: 'lr' | 'rl' | 'tb' | 'bt' // 行排版方向
  }
  angle: {
    reading: { inline: number; block: number } // 阅读方向角度
    angle: number // 整体旋转角
  }
}

/**
 * OCR 配置选项
 */
export interface OCRConfig {
  detRatio?: number // 检测缩放比例，默认 1.0
  recImgHeight?: number // 识别图片高度，默认 48
  enableSpaceOptimize?: boolean // 是否启用空格优化，默认 true
  enableDocCls?: boolean // 是否启用文档方向识别，默认 false
}

/**
 * OCR 管理器构造选项
 */
export interface OCRManagerOptions {
  resourcePath: string // OCR 资源文件路径
  isDev?: boolean // 是否为开发环境，默认 false
}

/**
 * OCR 管理器类
 * 基于 esearch-ocr 和 onnxruntime-node 实现的 OCR 识别工具
 */
export class OCRManager {
  private static instance: OCRManager | null = null
  private ocrInstance: any = null
  private isInitialized = false
  private resourcePath = ''
  private isDev = false

  private constructor(options: OCRManagerOptions) {
    this.resourcePath = options.resourcePath
    this.isDev = options.isDev || false
  }

  /**
   * 获取 OCR 管理器单例
   * @param options 构造选项，仅在首次调用时有效
   */
  public static getInstance(options?: OCRManagerOptions): OCRManager {
    if (!OCRManager.instance) {
      if (!options) {
        throw new Error('首次调用 getInstance 时必须提供 options 参数')
      }
      OCRManager.instance = new OCRManager(options)
    }
    return OCRManager.instance
  }

  /**
   * 初始化 OCR 引擎
   */
  public async initialize(config: OCRConfig = {}): Promise<void> {
    if (this.isInitialized) {
      logger.info('OCR 引擎已经初始化')
      return
    }

    try {
      logger.info('开始初始化 OCR 引擎...')

      // 设置 Node.js 环境的 canvas 支持
      setOCREnv({
        canvas: (w: number, h: number) => createCanvas(w, h),
        imageData: createImageData,
      })

      // 检查资源文件是否存在
      const detModelPath = path.join(this.resourcePath, 'ppocr_det.onnx')
      const recModelPath = path.join(this.resourcePath, 'ppocr_rec.onnx')
      const keysPath = path.join(this.resourcePath, 'ppocr_keys_v1.txt')
      
      await this.checkResourceFiles([detModelPath, recModelPath, keysPath])

      // 读取字典文件
      const decodeDic = await fs.readFile(keysPath, 'utf-8')

      // 初始化 OCR 配置
      const initConfig = {
        ort,
        det: {
          input: detModelPath,
          ratio: config.detRatio || 1.0,
          on: (result: any) => {
            logger.debug('检测完成:', result.length, '个文字区域')
          }
        },
        rec: {
          input: recModelPath,
          decodeDic: decodeDic.trim(),
          imgh: config.recImgHeight || 48,
          on: (index: number, _result: any, total: number) => {
            logger.debug(`识别进度: ${index + 1}/${total}`)
          },
          optimize: {
            space: config.enableSpaceOptimize !== false
          }
        },
        dev: this.isDev // 开发环境启用调试
      }

      // 如果启用文档方向识别，添加相关配置
      if (config.enableDocCls) {
        // 注意：需要额外的文档方向识别模型文件
        const clsModelPath = path.join(this.resourcePath, 'ppocr_cls.onnx')
        try {
          await fs.access(clsModelPath)
          ;(initConfig as any).docCls = {
            input: clsModelPath
          }
          logger.info('启用文档方向识别')
        } catch (error) {
          logger.warn('文档方向识别模型不存在，跳过该功能')
        }
      }

      // 初始化 OCR 实例
      this.ocrInstance = await init(initConfig)
      this.isInitialized = true

      logger.info('OCR 引擎初始化成功')
    } catch (error) {
      logger.error('OCR 引擎初始化失败:', error)
      throw new Error(`OCR 引擎初始化失败: ${error}`)
    }
  }

  /**
   * 检查资源文件是否存在
   */
  private async checkResourceFiles(filePaths: string[]): Promise<void> {
    for (const filePath of filePaths) {
      try {
        await fs.access(filePath)
        logger.debug(`资源文件存在: ${filePath}`)
      } catch (error) {
        throw new Error(`资源文件不存在: ${filePath}`)
      }
    }
  }

  /**
   * 执行 OCR 识别
   * @param imageInput 图片输入，支持 base64 字符串、文件路径、Buffer 等
   * @returns OCR 识别结果
   */
  public async recognize(imageInput: string | Buffer): Promise<OCROutput> {
    if (!this.isInitialized || !this.ocrInstance) {
      throw new Error('OCR 引擎未初始化，请先调用 initialize() 方法')
    }

    try {
      logger.debug('开始 OCR 识别...')

      // 将输入转换为 ImageData
      const imageData = await this.convertToImageData(imageInput)

      // 执行 OCR 识别
      const result = await this.ocrInstance.ocr(imageData)

      logger.debug('OCR 识别完成，识别到', result.src?.length || 0, '个文字区域')

      return result as OCROutput
    } catch (error) {
      logger.error('OCR 识别失败:', error)
      throw new Error(`OCR 识别失败: ${error}`)
    }
  }

  /**
   * 仅执行文字检测（不识别文字内容）
   */
  public async detect(imageInput: string | Buffer): Promise<any> {
    if (!this.isInitialized || !this.ocrInstance) {
      throw new Error('OCR 引擎未初始化，请先调用 initialize() 方法')
    }

    try {
      logger.debug('开始文字检测...')

      // 将输入转换为 ImageData
      const imageData = await this.convertToImageData(imageInput)

      const result = await this.ocrInstance.det(imageData)
      logger.debug('文字检测完成，检测到', result?.length || 0, '个文字区域')

      return result
    } catch (error) {
      logger.error('文字检测失败:', error)
      throw new Error(`文字检测失败: ${error}`)
    }
  }

  /**
   * 将输入转换为 ImageData
   */
  private async convertToImageData(imageInput: string | Buffer): Promise<any> {
    try {
      let image: any

      if (Buffer.isBuffer(imageInput)) {
        // 如果是 Buffer，直接使用 loadImage
        image = await loadImage(imageInput)
      } else if (imageInput.startsWith('data:image/')) {
        // 如果是 base64 格式，直接使用 loadImage
        image = await loadImage(imageInput)
      } else if (imageInput.startsWith('/') || imageInput.includes(':\\')) {
        // 如果是文件路径，读取文件
        image = await loadImage(imageInput)
      } else {
        // 假设是 base64 字符串，添加前缀
        const dataUrl = `data:image/png;base64,${imageInput}`
        image = await loadImage(dataUrl)
      }

      // 创建 canvas 并绘制图片
      const canvas = createCanvas(image.width, image.height)
      const ctx = canvas.getContext('2d')
      ctx.drawImage(image, 0, 0)

      // 获取 ImageData (使用 any 类型避免类型冲突)
      return ctx.getImageData(0, 0, image.width, image.height) as any
    } catch (error) {
      logger.error('转换 ImageData 失败:', error)
      throw new Error(`转换 ImageData 失败: ${error}`)
    }
  }

  /**
   * 获取识别结果的纯文本
   */
  public extractText(result: OCROutput): string {
    if (!result.parragraphs || result.parragraphs.length === 0) {
      return ''
    }

    return result.parragraphs.map((item) => item.text).join('\n')
  }

  /**
   * 静态方法：获取识别结果的纯文本
   */
  public static extractText(result: OCROutput): string {
    if (!result.parragraphs || result.parragraphs.length === 0) {
      return ''
    }

    return result.parragraphs.map((item) => item.text).join('\n')
  }



  /**
   * 销毁 OCR 实例
   */
  public destroy(): void {
    if (this.ocrInstance) {
      this.ocrInstance = null
      this.isInitialized = false
      logger.info('OCR 引擎已销毁')
    }
  }

  /**
   * 检查是否已初始化
   */
  public get initialized(): boolean {
    return this.isInitialized
  }

  /**
   * 获取资源路径
   */
  public get resourcesPath(): string {
    return this.resourcePath
  }
}

// 导出单例实例
// export const ocrManager = OCRManager.getInstance()
