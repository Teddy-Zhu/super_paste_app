import { defineStore } from 'pinia'
import { createDefaultSettings } from '@shared/preferences'

export const useSettingsStore = defineStore('settings', {
  state: createDefaultSettings,
  persist: true
})

export type SettingsStore = ReturnType<typeof useSettingsStore>
