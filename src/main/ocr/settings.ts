import type { ClipboardManagerConfig } from '../features/clipboard/types/clipboard-types'
import { STORE_KEY } from '../config/common'
import { getOCRModel, type OCRSettings } from '../../shared/ocr-models'

export const OCR_SETTING_KEYS: Record<keyof OCRSettings, string> = {
  enableLocalOCR: STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR,
  ocrMode: STORE_KEY.SETTINGS_OCR_MODE,
  ocrLocalModel: STORE_KEY.SETTINGS_OCR_LOCAL_MODEL,
  ocrRemoteUrl: STORE_KEY.SETTINGS_OCR_REMOTE_URL,
  ocrRemoteExtraParams: STORE_KEY.SETTINGS_OCR_REMOTE_EXTRA_PARAMS,
  ocrRemoteImageFieldName: STORE_KEY.SETTINGS_OCR_REMOTE_IMAGE_FIELD_NAME,
  ocrRemoteRequestFormat: STORE_KEY.SETTINGS_OCR_REMOTE_REQUEST_FORMAT,
  ocrRemoteResultPath: STORE_KEY.SETTINGS_OCR_REMOTE_RESULT_PATH
}

export function parseOCRSettings(value: unknown): OCRSettings {
  if (!value || typeof value !== 'object') throw new Error('OCR 配置无效')
  const input = value as Record<string, unknown>
  if (
    typeof input.enableLocalOCR !== 'boolean' ||
    (input.ocrMode !== 'local' && input.ocrMode !== 'remote') ||
    (input.ocrRemoteRequestFormat !== 'json' && input.ocrRemoteRequestFormat !== 'form')
  )
    throw new Error('OCR 配置无效')
  const model = getOCRModel(input.ocrLocalModel)
  const stringFields = [
    'ocrRemoteUrl',
    'ocrRemoteExtraParams',
    'ocrRemoteImageFieldName',
    'ocrRemoteResultPath'
  ] as const
  for (const field of stringFields)
    if (typeof input[field] !== 'string') throw new Error('OCR 配置字段无效: ' + field)
  const settings: OCRSettings = {
    enableLocalOCR: input.enableLocalOCR,
    ocrMode: input.ocrMode as OCRSettings['ocrMode'],
    ocrLocalModel: model.id,
    ocrRemoteUrl: (input.ocrRemoteUrl as string).trim(),
    ocrRemoteExtraParams: (input.ocrRemoteExtraParams as string).trim() || '{}',
    ocrRemoteImageFieldName: (input.ocrRemoteImageFieldName as string).trim() || 'image',
    ocrRemoteRequestFormat: input.ocrRemoteRequestFormat as OCRSettings['ocrRemoteRequestFormat'],
    ocrRemoteResultPath: (input.ocrRemoteResultPath as string).trim() || 'data.result'
  }
  if (settings.ocrMode === 'remote') {
    const params: unknown = JSON.parse(settings.ocrRemoteExtraParams)
    if (!params || typeof params !== 'object' || Array.isArray(params))
      throw new Error('额外参数必须是 JSON 对象')
    if (settings.ocrRemoteUrl) {
      const url = new URL(settings.ocrRemoteUrl)
      if (!['http:', 'https:'].includes(url.protocol))
        throw new Error('远程 OCR 地址必须使用 HTTP 或 HTTPS')
    } else if (settings.enableLocalOCR) throw new Error('远程 OCR 地址不能为空')
  }
  return settings
}

export function toClipboardOCRConfig(
  settings: OCRSettings
): Pick<
  ClipboardManagerConfig,
  'enableLocalOCR' | 'ocrMode' | 'ocrLocalModel' | 'ocrRemoteConfig'
> {
  return {
    enableLocalOCR: settings.enableLocalOCR,
    ocrMode: settings.ocrMode,
    ocrLocalModel: settings.ocrLocalModel,
    ocrRemoteConfig: {
      url: settings.ocrRemoteUrl,
      extraParams: settings.ocrRemoteExtraParams,
      imageFieldName: settings.ocrRemoteImageFieldName,
      requestFormat: settings.ocrRemoteRequestFormat,
      resultPath: settings.ocrRemoteResultPath
    }
  }
}
