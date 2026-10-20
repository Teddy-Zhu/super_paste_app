import { defineStore } from 'pinia'
import { createDefaultSyncPreferences } from '@shared/preferences'

export const useSyncStore = defineStore('sync', {
  state: createDefaultSyncPreferences,
  persist: true
})
