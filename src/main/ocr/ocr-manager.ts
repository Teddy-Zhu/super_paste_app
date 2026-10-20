import { init, setOCREnv, type initType, type InitOcrBase, type detResultType } from 'esearch-ocr'
import * as ort from 'onnxruntime-node'
import { promises as fs } from 'node:fs'
import { createCanvas, loadImage, createImageData } from 'canvas'
import type { OCRModelId } from '../../shared/ocr-models'
import { getInstalledOCRModel } from './model-store'
import logger from '../utils/logger'

export type OCROutput = Awaited<ReturnType<initType['ocr']>>

export interface OCRConfig {
  detRatio?: number
  recImgHeight?: number
  enableSpaceOptimize?: boolean
}

export interface OCRManagerOptions {
  resourcePath: string
  modelId: OCRModelId
}

export class OCRManager {
  private ocrInstance: initType | null = null
  private sessions = new Map<string, ort.InferenceSession>()
  private operations: Promise<void> = Promise.resolve()
  private modelDirectory = ''

  constructor(private readonly options: OCRManagerOptions) {}

  // eSearch-OCR 的图像处理共享运行环境，识别与释放必须按顺序执行。
  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.operations.then(operation)
    this.operations = result.then(
      () => undefined,
      () => undefined
    )
    return result
  }

  public initialize(config: OCRConfig = {}): Promise<void> {
    return this.enqueue(async () => {
      if (this.ocrInstance) return
      const installed = await getInstalledOCRModel(
        this.options.resourcePath,
        this.options.modelId,
        true
      )
      if (!installed) throw new Error('本地 OCR 模型未下载或已损坏，请在 OCR 设置中下载模型')
      setOCREnv({ canvas: createCanvas, imageData: createImageData, dev: false, log: false })
      try {
        const sessionOptions: ort.InferenceSession.SessionOptions = {
          executionProviders: ['cpu'],
          graphOptimizationLevel: 'all'
        }
        for (const file of [installed.det, installed.rec]) {
          this.sessions.set(file, await ort.InferenceSession.create(file, sessionOptions))
        }
        const metadata = this.sessions.get(installed.rec)!.inputMetadata[0]
        const height = metadata?.isTensor ? metadata.shape[2] : undefined
        // 复用已加载的原生会话，同时兼容上游声明的 onnxruntime-common 类型。
        const runtime = {
          ...ort,
          InferenceSession: {
            create: async (input: unknown): Promise<ort.InferenceSession> => {
              const session = typeof input === 'string' ? this.sessions.get(input) : undefined
              if (!session) throw new Error('未知的 OCR 模型会话')
              return session
            }
          }
        } as unknown as InitOcrBase['ort']
        this.ocrInstance = await init({
          ort: runtime,
          det: { input: installed.det, ratio: config.detRatio ?? 1 },
          rec: {
            input: installed.rec,
            // 保留字典中的空白字符，避免改变词表索引。
            decodeDic: (await fs.readFile(installed.dictionary, 'utf8')).replace(/^\uFEFF/, ''),
            imgh: config.recImgHeight ?? (typeof height === 'number' && height > 0 ? height : 48),
            optimize: { space: config.enableSpaceOptimize ?? installed.model.optimizeSpace }
          },
          dev: false,
          log: false
        })
        this.modelDirectory = installed.directory
        logger.info('[OCR] 模型加载完成:', installed.model.id)
      } catch (error) {
        try {
          await this.releaseSessions()
        } catch (releaseError) {
          logger.error('[OCR] 初始化失败后释放模型失败:', releaseError)
        }
        throw error
      }
    })
  }

  public recognize(imageInput: string | Buffer): Promise<OCROutput> {
    return this.enqueue(async () => {
      if (!this.ocrInstance) throw new Error('OCR 引擎未初始化')
      return this.ocrInstance.ocr(await this.convertToImageData(imageInput))
    })
  }

  public detect(imageInput: string | Buffer): Promise<detResultType> {
    return this.enqueue(async () => {
      if (!this.ocrInstance) throw new Error('OCR 引擎未初始化')
      return this.ocrInstance.det(await this.convertToImageData(imageInput))
    })
  }

  private async convertToImageData(imageInput: string | Buffer): Promise<ImageData> {
    let source: string | Buffer = imageInput
    if (typeof source === 'string' && !source.startsWith('data:')) {
      try {
        await fs.access(source)
      } catch (error) {
        if (!['ENOENT', 'ENAMETOOLONG'].includes((error as NodeJS.ErrnoException).code || ''))
          throw error
        source = Buffer.from(source, 'base64')
      }
    }
    const image = await loadImage(source)
    if (!image.width || !image.height) throw new Error('图片尺寸无效')
    const canvas = createCanvas(image.width, image.height)
    const context = canvas.getContext('2d')
    context.drawImage(image, 0, 0)
    return context.getImageData(0, 0, image.width, image.height) as unknown as ImageData
  }

  public extractText(result: OCROutput): string {
    const lines = result.parragraphs.length ? result.parragraphs : result.src
    return lines.map((item) => item.text).join('\n')
  }

  private async releaseSessions(): Promise<void> {
    const results = await Promise.allSettled(
      [...this.sessions.values()].map((session) => session.release())
    )
    this.sessions.clear()
    const errors = results
      .filter((result) => result.status === 'rejected')
      .map((result) => result.reason)
    if (errors.length) throw new AggregateError(errors, '释放 OCR 模型失败')
  }

  public destroy(): Promise<void> {
    return this.enqueue(async () => {
      this.ocrInstance = null
      this.modelDirectory = ''
      await this.releaseSessions()
    })
  }

  public get initialized(): boolean {
    return this.ocrInstance !== null
  }

  public get resourcesPath(): string {
    return this.modelDirectory
  }
}
