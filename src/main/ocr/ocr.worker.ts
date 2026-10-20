import { parentPort } from 'node:worker_threads'
import { ocrService, type OCRServiceOptions } from '../services/ocr-service'
import { BaseWorker } from '../workers/base-worker'
import type { WorkerMessage, WorkerRequest, WorkerResponse } from '../types/worker-types'

class OCRWorker extends BaseWorker {
  protected getWorkerName(): string {
    return 'OCR'
  }

  public async handleMessage(message: WorkerMessage): Promise<void> {
    if (message.type !== 'request') return
    const request = message as WorkerRequest
    const response: WorkerResponse = {
      id: `response-${request.id}`,
      type: 'response',
      requestId: request.id,
      success: true,
      timestamp: Date.now()
    }
    try {
      switch (request.method) {
        case 'initialize': {
          const options = request.params as OCRServiceOptions
          await ocrService.initialize({
            ...options,
            onAIUsage: (entry) => this.sendEvent('ai-ocr-usage', entry)
          })
          break
        }
        case 'recognizeText': {
          const result = await ocrService.recognizeText(Buffer.from(request.params as Uint8Array))
          // 只返回识别结果，避免把本地推理中间数据复制回剪贴板线程。
          response.data = {
            success: result.success,
            text: result.text,
            confidence: result.confidence,
            error: result.error
          }
          break
        }
        default:
          throw new Error('未知 OCR Worker 方法')
      }
    } catch (error) {
      response.success = false
      response.error = error instanceof Error ? error.message : String(error)
    }
    parentPort?.postMessage(response)
  }

  public ready(): void {
    this.sendWorkerReadyEvent()
  }
}

const worker = new OCRWorker()
parentPort?.on('message', (message: WorkerMessage) => {
  void worker.handleMessage(message)
})
worker.ready()
