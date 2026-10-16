import { createPinia } from 'pinia'
import { createPersistedState } from 'pinia-plugin-persistedstate'
import { invoke } from '@tauri-apps/api/core'
import { createFieldChangePlugin } from './fieldChangePlugin'

const store = createPinia()

// 使用 localStorage 作为持久化存储（先注册，确保 $persist 在字段插件运行前就绪）
store.use(
  createPersistedState({
    storage: localStorage
  })
)

/**
 * settings store 字段（camelCase） -> 后端配置键（snake_case） 映射。
 * 后端为权威源：字段变更经此映射调用 set_config_value 落库并触发副作用生效。
 */
export const SETTINGS_KEY_MAP: Record<string, string> = {
  autoStart: 'auto_start',
  overlayPosition: 'overlay_position',
  globalShortcut: 'global_shortcut',
  maxHistorySize: 'max_history_size',
  maxKeepTime: 'max_keep_days',
  clipboardListening: 'clipboard_listening',
  enableLocalOCR: 'enable_local_ocr',
  ocrMode: 'ocr_mode',
  ocrRemoteUrl: 'ocr_remote_url',
  ocrRemoteExtraParams: 'ocr_remote_extra_params',
  ocrRemoteImageFieldName: 'ocr_remote_image_field_name',
  ocrRemoteRequestFormat: 'ocr_remote_request_format',
  ocrRemoteResultPath: 'ocr_remote_result_path'
}

// 后端键 -> 前端字段 反向映射（载入时回填用）
export const BACKEND_TO_SETTINGS_KEY: Record<string, string> = Object.fromEntries(
  Object.entries(SETTINGS_KEY_MAP).map(([camel, snake]) => [snake, camel])
)

// 将 settings store 的字段变更持久化到后端（并由后端应用副作用使其生效）
store.use(
  createFieldChangePlugin(({ storeId, key, newValue }) => {
    if (storeId !== 'settings') return
    const backendKey = SETTINGS_KEY_MAP[key]
    if (!backendKey) return

    const value = typeof newValue === 'string' ? newValue : String(newValue)
    invoke('set_config_value', { key: backendKey, value }).catch((err) => {
      console.error('[settings] 持久化配置到后端失败:', key, err)
    })
  })
)

export default store
