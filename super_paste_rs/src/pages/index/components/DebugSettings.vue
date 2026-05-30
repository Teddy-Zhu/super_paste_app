<script setup lang="ts">
import { send } from '@renderer/api'
import { clipboard } from '@renderer/api/renderer-api'

// Props
interface Props {
  settingsStore: any
}

const props = defineProps<Props>()

// 打开覆盖层窗口
const openOverlayWindow = (): void => {
  send('open-overlay-window')
}

// 切换剪贴板监听
const toggleClipboardListening = async () => {
  try {
    const newState = !props.settingsStore.clipboardListening

    if (newState) {
      const success = await clipboard.startListening()
      if (success) {
        props.settingsStore.clipboardListening = true
        console.log('剪贴板监听已开启')
      } else {
        console.error('开启剪贴板监听失败')
      }
    } else {
      const success = await clipboard.stopListening()
      if (success) {
        props.settingsStore.clipboardListening = false
        console.log('剪贴板监听已关闭')
      } else {
        console.error('关闭剪贴板监听失败')
      }
    }
  } catch (error) {
    console.error('切换剪贴板监听状态失败:', error)
  }
}

// 同步剪贴板监听状态
const syncClipboardListeningStatus = async () => {
  try {
    const status = await clipboard.getStatus()
    if (status && typeof status.isListening === 'boolean') {
      props.settingsStore.clipboardListening = status.isListening
      console.log('剪贴板监听状态已同步:', status.isListening)
    }
  } catch (error) {
    console.error('获取剪贴板监听状态失败:', error)
  }
}

// 导出 heapdump
const exportHeapdump = async () => {
  try {
    const result = await window.electron.ipcRenderer.invoke('export-heapdump')
    if (result.success) {
      // 构建详细的成功消息
      let message = result.message
      if (result.files && result.files.length > 1) {
        message += '\n\n导出的文件：'
        result.files.forEach((file: string, index: number) => {
          const filename = file.split(/[/\\]/).pop() || file
          message += `\n${index + 1}. ${filename}`
        })
      } else if (result.mainFile) {
        const filename = result.mainFile.split(/[/\\]/).pop() || result.mainFile
        message += `\n文件名: ${filename}`
      }

      alert(message)
      console.log('heapdump 导出成功:', result)
    } else {
      alert(`导出失败: ${result.error}`)
      console.error('heapdump 导出失败:', result.error)
    }
  } catch (error) {
    console.error('导出 heapdump 失败:', error)
    alert('导出失败: ' + (error as Error).message)
  }
}

// 组件挂载时同步剪贴板状态
onMounted(() => {
  syncClipboardListeningStatus()
})
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
          <button class="test-button" @click="openOverlayWindow">打开测试</button>
        </div>
      </div>

      <!-- 导出内存快照 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">导出内存快照</h4>
          <p class="setting-description">导出当前应用的内存快照文件到桌面，用于内存分析和调试</p>
        </div>
        <div class="setting-control">
          <button class="test-button" @click="exportHeapdump">导出 Heapdump</button>
        </div>
      </div>
    </div>
  </div>
</template>
