import { DEFAULT_OCR_MODEL_ID, type OCRModelId, type OCRSettings } from './ocr-models'
import { createDefaultAIOCRConfigs } from './ai-ocr'
import { DEFAULT_COLOR_MODE, type ColorMode } from './color-mode'

const defaultSettings = {
  colorMode: DEFAULT_COLOR_MODE as ColorMode,
  autoStart: false,
  overlayPosition: 'bottom' as 'top' | 'bottom' | 'left' | 'right',
  globalShortcut: '',
  maxHistorySize: 100,
  maxKeepTime: -1,
  clipboardListening: true,
  enableLocalOCR: false,
  ocrMode: 'local' as OCRSettings['ocrMode'],
  ocrLocalModel: DEFAULT_OCR_MODEL_ID as OCRModelId,
  ocrRemoteUrl: '',
  ocrRemoteExtraParams: '{}',
  ocrRemoteImageFieldName: 'image',
  ocrRemoteRequestFormat: 'json' as 'json' | 'form',
  ocrRemoteResultPath: 'data.result',
  ocrAIConfigs: createDefaultAIOCRConfigs()
}

const defaultSyncPreferences = {
  enabled: false,
  provider: '',
  interval: 30 * 60000,
  direction: 'bidirectional',
  params: {
    webdav: {
      serverUrl: '',
      username: '',
      password: '',
      remotePath: '',
      timeout: 30000,
      useHttps: false
    }
  }
}

export function createDefaultSettings(): typeof defaultSettings {
  return structuredClone(defaultSettings)
}

export function createDefaultSyncPreferences(): typeof defaultSyncPreferences {
  return structuredClone(defaultSyncPreferences)
}
