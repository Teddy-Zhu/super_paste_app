import type { AIOCRConfigs, AIOCRProvider } from './ai-ocr'
import modelManifest from '../../resources/ocr-models.json'

export interface OCRModel {
  id: string
  name: string
  group: string
  description: string
  archive: string
  size: number
  sha256: string | null
  optimizeSpace: boolean
}

export const OCR_MODELS = modelManifest.models as readonly OCRModel[]
export type OCRModelId = OCRModel['id']
export const DEFAULT_OCR_MODEL_ID: OCRModelId = modelManifest.defaultModelId

export function getOCRModel(id: unknown): OCRModel {
  const model = OCR_MODELS.find((item) => item.id === id)
  if (!model) throw new Error('不支持的 OCR 模型')
  return model
}

export function getOCRModelURL(model: OCRModel): string {
  return modelManifest.releaseUrl + model.archive
}

export function getOCRModelRevision(model: OCRModel): string {
  return model.archive + ':' + (model.sha256 || model.size)
}

export type OCRModelStage = 'downloading' | 'extracting' | 'preparing-detector' | 'verifying'
export interface OCRModelStatus {
  id: OCRModelId
  state: 'not-downloaded' | 'downloading' | 'ready' | 'error'
  stage?: OCRModelStage
  receivedBytes: number
  totalBytes: number
  error?: string
}

export interface OCRSettings {
  enableLocalOCR: boolean
  ocrMode: 'local' | 'remote' | AIOCRProvider
  ocrLocalModel: OCRModelId
  ocrRemoteUrl: string
  ocrRemoteExtraParams: string
  ocrRemoteImageFieldName: string
  ocrRemoteRequestFormat: 'json' | 'form'
  ocrRemoteResultPath: string
  ocrAIConfigs: AIOCRConfigs
}

export const OCR_MODEL_STAGE_LABELS: Record<OCRModelStage, string> = {
  downloading: '下载中',
  extracting: '解压中',
  'preparing-detector': '准备文字检测模型',
  verifying: '校验中'
}
