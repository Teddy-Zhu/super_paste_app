import { defineStore } from 'pinia'

export const useSyncStore = defineStore('sync', {
  state: () => ({
    enabled: false,
    provider: '',
    interval: 3000000,
    direction: 'bidirectional',
    configOnly: false,
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
  }),
  getters: {},
  actions: {},
  persist: true
})
