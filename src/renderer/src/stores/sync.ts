import { defineStore } from 'pinia'

export const useSettingsStore = defineStore('settings',{
  state: () => ({
    autoStart: false,
    globalShortcut: '', // 默认为空，用户需要手动设置
    maxHistorySize: 100,
    maxKeepTime: -1, // 最大保存时间（天），-1 表示不限制
    clipboardListening: true, // 剪贴板监听状态，默认开启
  }),
  getters: {

  },
  actions: {
    
  },
  persist: true
})
