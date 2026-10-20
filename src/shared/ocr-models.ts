const RELEASE_URL = 'https://github.com/xushengfeng/eSearch-OCR/releases/download/4.0.0/'

export const OCR_MODELS = [
  {
    id: 'ppocr-v6-tiny',
    name: 'PP-OCR v6 tiny',
    group: 'PP-OCR v6',
    description: '轻量模型',
    archive: 'ppocr_v6_tiny.zip',
    size: 5695955,
    sha256: '23567e36aca7994dd895ed4c28ca2dec0953aa579e7b19b0817a3748e5a372cf',
    optimizeSpace: false
  },
  {
    id: 'ppocr-v6-small',
    name: 'PP-OCR v6 small',
    group: 'PP-OCR v6',
    description: '均衡模型',
    archive: 'ppocr_v6_small.zip',
    size: 26028589,
    sha256: '2e36eb4c59f57cd53d2324665887c0b20f08fad7002e6b2237045a55d590fb92',
    optimizeSpace: false
  },
  {
    id: 'ppocr-v6-medium',
    name: 'PP-OCR v6 medium',
    group: 'PP-OCR v6',
    description: '较大模型',
    archive: 'ppocr_v6_medium.zip',
    size: 100061146,
    sha256: '5bf6f42a775a8dd91c49ec588f628dd9438709720043cf578233c55c0838c262',
    optimizeSpace: false
  },
  {
    id: 'ppocr-v5-mobile',
    name: 'PP-OCR v5 mobile',
    group: 'PP-OCR v5',
    description: '中英文移动版',
    archive: 'ppocr_v5_mobile.zip',
    size: 18644132,
    sha256: null,
    optimizeSpace: false
  },
  {
    id: 'ppocr-v5-server',
    name: 'PP-OCR v5 server',
    group: 'PP-OCR v5',
    description: '中英文服务器版',
    archive: 'ppocr_v5_server.zip',
    size: 155631244,
    sha256: null,
    optimizeSpace: false
  },
  {
    id: 'ppocr-v4',
    name: 'PP-OCR v4',
    group: 'PP-OCR v4 / v3',
    description: '中英文混合',
    archive: 'ch.zip',
    size: 14365264,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'ppocr-v4-doc',
    name: 'PP-OCR v4 doc',
    group: 'PP-OCR v4 / v3',
    description: '中英文高精度版',
    archive: 'ch_v4_doc.zip',
    size: 82735471,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'ppocr-v3',
    name: 'PP-OCR v3',
    group: 'PP-OCR v4 / v3',
    description: '中英文混合',
    archive: 'ch_v3.zip',
    size: 11771073,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'chinese-cht',
    name: '繁体中文',
    group: '其他语言',
    description: '繁体中文模型',
    archive: 'chinese_cht.zip',
    size: 10126839,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'english',
    name: '英文',
    group: '其他语言',
    description: '英文模型',
    archive: 'en.zip',
    size: 8146377,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'japanese',
    name: '日文',
    group: '其他语言',
    description: '日文模型',
    archive: 'japan.zip',
    size: 9199755,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'korean',
    name: '韩文',
    group: '其他语言',
    description: '韩文模型',
    archive: 'korean.zip',
    size: 8952561,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'telugu',
    name: '泰卢固文',
    group: '其他语言',
    description: '泰卢固文模型',
    archive: 'te.zip',
    size: 8102565,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'kannada',
    name: '卡纳达文',
    group: '其他语言',
    description: '卡纳达文模型',
    archive: 'ka.zip',
    size: 8286396,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'tamil',
    name: '泰米尔文',
    group: '其他语言',
    description: '泰米尔文模型',
    archive: 'ta.zip',
    size: 8354625,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'latin',
    name: '拉丁字母',
    group: '其他语言',
    description: '拉丁字母模型',
    archive: 'latin.zip',
    size: 8090341,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'arabic',
    name: '阿拉伯字母',
    group: '其他语言',
    description: '阿拉伯字母模型',
    archive: 'arabic.zip',
    size: 8124448,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'cyrillic',
    name: '西里尔字母',
    group: '其他语言',
    description: '西里尔字母模型',
    archive: 'cyrillic.zip',
    size: 8230331,
    sha256: null,
    optimizeSpace: true
  },
  {
    id: 'devanagari',
    name: '梵文字母',
    group: '其他语言',
    description: '梵文字母模型',
    archive: 'devanagari.zip',
    size: 8270883,
    sha256: null,
    optimizeSpace: true
  }
] as const

export type OCRModel = (typeof OCR_MODELS)[number]
export type OCRModelId = OCRModel['id']
export const DEFAULT_OCR_MODEL_ID: OCRModelId = 'ppocr-v6-tiny'

export function getOCRModel(id: unknown): OCRModel {
  const model = OCR_MODELS.find((item) => item.id === id)
  if (!model) throw new Error('不支持的 OCR 模型')
  return model
}

export function getOCRModelURL(model: OCRModel): string {
  return RELEASE_URL + model.archive
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
  ocrMode: 'local' | 'remote'
  ocrLocalModel: OCRModelId
  ocrRemoteUrl: string
  ocrRemoteExtraParams: string
  ocrRemoteImageFieldName: string
  ocrRemoteRequestFormat: 'json' | 'form'
  ocrRemoteResultPath: string
}

export const OCR_MODEL_STAGE_LABELS: Record<OCRModelStage, string> = {
  downloading: '下载中',
  extracting: '解压中',
  'preparing-detector': '准备文字检测模型',
  verifying: '校验中'
}
