<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { useSettingsStore } from '@renderer/stores/common'
import { applyColorScheme } from './utils/color-mode'

const settingsStore = useSettingsStore()
const systemPrefersDark = window.matchMedia('(prefers-color-scheme: dark)')
const isSystemDark = ref(systemPrefersDark.matches)

const updateColorScheme = (): void => {
  applyColorScheme(settingsStore.colorMode, isSystemDark.value)
}

const handleSystemThemeChange = (event: MediaQueryListEvent): void => {
  isSystemDark.value = event.matches
}

watch(() => settingsStore.colorMode, updateColorScheme, { immediate: true })
watch(isSystemDark, () => {
  if (settingsStore.colorMode === 'system') updateColorScheme()
})
systemPrefersDark.addEventListener('change', handleSystemThemeChange)
onUnmounted(() => systemPrefersDark.removeEventListener('change', handleSystemThemeChange))
</script>

<template>
  <router-view />
</template>
