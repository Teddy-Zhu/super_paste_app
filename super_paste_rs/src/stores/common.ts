import { defineStore } from 'pinia'

export const useSettingsStore = defineStore('settings', {
  state: () => ({
    autoStart: false,
    overlayPosition: 'bottom' as 'top' | 'bottom' | 'left' | 'right',
    globalShortcut: '', // 默认为空，用户需要手动设置
    maxHistorySize: 100,
    maxKeepTime: -1, // 最大保存时间（天），-1 表示不限制
    clipboardListening: true, // 剪贴板监听状态，默认开启
    enableLocalOCR: false, // 是否启用自动OCR，默认关闭
    ocrMode: 'local' as 'local' | 'remote',
    ocrRemoteUrl: '',
    ocrRemoteExtraParams: '{}',
    ocrRemoteImageFieldName: 'image',
    ocrRemoteRequestFormat: 'json' as 'json' | 'form',
    ocrRemoteResultPath: 'data.result'
  }),
  getters: {},
  actions: {},
  persist: true
})
