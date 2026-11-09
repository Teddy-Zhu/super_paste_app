<script setup lang="ts">
import type { SettingsStore } from '@renderer/stores/common'
import { send } from '@renderer/api'
import { invoke } from '@renderer/api/ipc'
import { showMessage } from '@renderer/api/dialog'

// Props
interface Props {
  settingsStore: SettingsStore
}

const props = defineProps<Props>()

const openOverlayWindow = (event: PointerEvent | MouseEvent): void => {
  // Pointer activation must precede blur confirmation; its later click is a duplicate.
  // Keyboard and assistive activation generate a click with no pointer detail.
  if (event.type === 'click' && event.detail > 0) return
  send('open-overlay-window')
}

// 切换剪贴板监听
const toggleClipboardListening = async (): Promise<void> => {
  try {
    const newState = !props.settingsStore.clipboardListening
    await props.settingsStore.$commit({ clipboardListening: newState })
  } catch (error) {
    console.error('切换剪贴板监听状态失败:', error)
  }
}

// 导出脱敏诊断信息
const exportDiagnostics = async (): Promise<void> => {
  try {
    const result = await invoke<{
      success: boolean
      message?: string
      file?: string
      error?: string
    }>('export-diagnostics', navigator.userAgent)
    if (result.success) {
      const filename = result.file?.split(/[/\\]/).pop()
      await showMessage([result.message, filename].filter(Boolean).join('\n'))
    } else if (result.message !== '已取消导出') {
      await showMessage(`导出失败: ${result.error || result.message || '未知错误'}`)
      console.error('诊断信息导出失败:', result.error || result.message)
    }
  } catch (error) {
    console.error('导出诊断信息失败:', error)
    await showMessage('导出失败: ' + (error as Error).message)
  }
}
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">调试设置</h3>
      <p class="section-subtitle">用于开发和调试的额外选项</p>
    </div>

    <div class="settings-grid">
      <!-- 剪贴板监听开关 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">剪贴板监听</h4>
          <p class="setting-description">控制是否监听剪贴板内容变化并保存历史记录</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input
              type="checkbox"
              :checked="settingsStore.clipboardListening"
              @change="toggleClipboardListening"
            />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- 测试覆盖层 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">测试覆盖层</h4>
          <p class="setting-description">打开覆盖层窗口进行测试</p>
        </div>
        <div class="setting-control">
          <button
            class="test-button"
            @pointerdown.left.prevent="openOverlayWindow"
            @click="openOverlayWindow"
          >
            打开测试
          </button>
        </div>
      </div>

      <!-- 导出诊断信息 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">导出诊断信息</h4>
          <p class="setting-description">导出应用运行状态和脱敏日志</p>
        </div>
        <div class="setting-control">
          <button class="test-button" @click="exportDiagnostics">导出诊断信息</button>
        </div>
      </div>
    </div>
  </div>
</template>
