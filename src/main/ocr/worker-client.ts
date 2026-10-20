import { BaseWorkerManager } from '../workers/base-worker-manager'
import type { OCRServiceOptions, OCRRecognizeResult } from '../services/ocr-service'
import type { AIOCRUsageEntry } from '../../shared/ai-ocr'
import { AI_OCR_REQUEST_TIMEOUT_MS } from '../../shared/ai-ocr'

// OCR 的图片处理和本地推理放在独立线程，避免占用剪贴板监听线程。
class OCRWorkerClient extends BaseWorkerManager {
  private serviceReady = false
  private onUsage: OCRServiceOptions['onAIUsage']

  constructor(workerPath: string) {
    super({ name: 'OCR', workerPath })
    this.on('ai-ocr-usage', (entry: AIOCRUsageEntry) => this.onUsage?.(entry))
    this.on('worker-error', () => {
      this.serviceReady = false
    })
    this.on('worker-exit', () => {
      this.serviceReady = false
    })
  }

  public get ready(): boolean {
    return this.serviceReady && this.isRunning()
  }

  public async initialize(options: OCRServiceOptions): Promise<void> {
    const { onAIUsage, ...config } = options
    this.onUsage = onAIUsage
    this.serviceReady = false
    await this.start()
    await this.call('initialize', config, AI_OCR_REQUEST_TIMEOUT_MS + 5000)
    this.serviceReady = true
  }

  public reinitialize(options: OCRServiceOptions): Promise<void> {
    return this.initialize(options)
  }

  public async recognizeText(image: Buffer): Promise<OCRRecognizeResult> {
    try {
      return await this.call<OCRRecognizeResult>(
        'recognizeText',
        image,
        AI_OCR_REQUEST_TIMEOUT_MS + 5000
      )
    } catch (error) {
      // 超时后结束执行线程，确保释放队列锁之前旧请求已退出。
      await this.stop()
      throw error
    }
  }

  public override async stop(): Promise<void> {
    this.serviceReady = false
    await super.stop()
  }
}

export { OCRWorkerClient }
