import { defineStore } from 'pinia'

export const useSettingsStore = defineStore('settings', {
  state: () => ({
    autoStart: false,
    overlayPosition: 'bottom' as 'top' | 'bottom' | 'left' | 'right',
    globalShortcut: '',
    maxHistorySize: 100,
    maxKeepTime: -1,
    clipboardListening: true,
    enableLocalOCR: false,
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
