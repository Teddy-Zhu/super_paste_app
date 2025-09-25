<script setup lang="ts">
import IconSettings from '~icons/mdi/settings'
import IconKeyboard from '~icons/mdi/keyboard'
import IconInformation from '~icons/mdi/information'
import IconTag from '~icons/mdi/tag'
import IconBug from '~icons/mdi/bug'
import IconSync from '~icons/mdi/sync'
import { useSettingsStore } from '@renderer/stores/common'

// 导入组件
import GeneralSettings from './components/GeneralSettings.vue'
import TagsManagement from './components/TagsManagement.vue'
import ShortcutSettings from './components/ShortcutSettings.vue'
import DebugSettings from './components/DebugSettings.vue'
import AboutSection from './components/AboutSection.vue'
import SyncSettings from './components/SyncSettings.vue'

// 导入API
import { onSimple } from '@renderer/api/ipc'
import { app } from '@renderer/api/renderer-api'
import { ref, onMounted, onUnmounted } from 'vue'

// 当前选中的设置项
const activeSection = ref('general')
const settingsStore = useSettingsStore()

// 重启确认对话框状态
const showRestartDialog = ref(false)
const restartDialogData = ref<{
  downloadedFiles: string[]
  message: string
} | null>(null)

// 设置项列表
const settingSections = [
  {
    id: 'general',
    name: '通用设置',
    icon: IconSettings
  },
  {
    id: 'sync',
    name: '同步设置',
    icon: IconSync
  },
  {
    id: 'tags',
    name: '标签管理',
    icon: IconTag
  },
  {
    id: 'shortcuts',
    name: '快捷键',
    icon: IconKeyboard
  },
  {
    id: 'debug',
    name: '调试设置',
    icon: IconBug
  },
  {
    id: 'about',
    name: '关于',
    icon: IconInformation
  }
]

// 切换设置项
const setActiveSection = (sectionId: string) => {
  activeSection.value = sectionId
}

// 处理同步需要重启事件
const handleSyncRequiresRestart = (data: { downloadedFiles: string[]; message: string }) => {
  console.log('收到同步需要重启通知:', data)
  restartDialogData.value = data
  showRestartDialog.value = true
}

// 确认重启应用
const confirmRestart = async () => {
  try {
    showRestartDialog.value = false
    const success = await app.restart()
    if (!success) {
      alert('重启应用失败，请手动重启应用以应用更改')
    }
  } catch (error) {
    console.error('重启应用失败:', error)
    alert('重启应用失败，请手动重启应用以应用更改')
  }
}

// 取消重启
const cancelRestart = () => {
  showRestartDialog.value = false
  restartDialogData.value = null
}

// IPC事件监听器清理函数
let restartRequiredUnsubscribe: (() => void) | null = null

onMounted(() => {
  // 设置IPC事件监听器
  restartRequiredUnsubscribe = onSimple('sync-requires-restart', handleSyncRequiresRestart)
})

onUnmounted(() => {
  // 清理IPC事件监听器
  if (restartRequiredUnsubscribe) {
    restartRequiredUnsubscribe()
    restartRequiredUnsubscribe = null
  }
})
</script>

<template>
  <div class="settings-container">
    <!-- 左侧边栏 -->
    <div class="sidebar">
      <div class="sidebar-header">
        <h2 class="app-title">Super Paste</h2>
        <p class="app-subtitle">设置</p>
      </div>

      <nav class="sidebar-nav">
        <div
          v-for="section in settingSections"
          :key="section.id"
          class="nav-item"
          :class="{ active: activeSection === section.id }"
          @click="setActiveSection(section.id)"
        >
          <component :is="section.icon" class="nav-icon" />
          <span class="nav-text">{{ section.name }}</span>
        </div>
      </nav>
    </div>

    <!-- 右侧内容区域 -->
    <div class="main-content">
      <!-- 通用设置 -->
      <GeneralSettings v-if="activeSection === 'general'" :settings-store="settingsStore" />

      <!-- 同步设置 -->
      <SyncSettings v-if="activeSection === 'sync'" />

      <!-- 标签管理 -->
      <TagsManagement v-if="activeSection === 'tags'" />

      <!-- 快捷键设置 -->
      <ShortcutSettings v-if="activeSection === 'shortcuts'" :settings-store="settingsStore" />

      <!-- 调试设置 -->
      <DebugSettings v-if="activeSection === 'debug'" :settings-store="settingsStore" />

      <!-- 关于 -->
      <AboutSection v-if="activeSection === 'about'" />
    </div>

    <!-- 重启确认对话框 -->
    <div v-if="showRestartDialog" class="restart-dialog-overlay" @click.self="cancelRestart">
      <div class="restart-dialog">
        <div class="restart-dialog-header">
          <h3 class="restart-dialog-title">同步完成</h3>
        </div>

        <div class="restart-dialog-body">
          <div class="restart-icon">
            <svg
              width="48"
              height="48"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <path d="M1 4l6 6-6 6" />
              <path d="M11 4l6 6-6 6" />
            </svg>
          </div>

          <p class="restart-message">{{ restartDialogData?.message }}</p>

          <div v-if="restartDialogData?.downloadedFiles?.length" class="downloaded-files">
            <p class="files-title">已下载的文件:</p>
            <ul class="files-list">
              <li v-for="file in restartDialogData.downloadedFiles" :key="file" class="file-item">
                {{ file }}
              </li>
            </ul>
          </div>

          <p class="restart-description">
            为了应用这些更改，需要重启应用。重启后所有新的数据和配置将生效。
          </p>
        </div>

        <div class="restart-dialog-footer">
          <button type="button" @click="cancelRestart" class="restart-btn restart-btn-cancel">
            稍后重启
          </button>
          <button type="button" @click="confirmRestart" class="restart-btn restart-btn-confirm">
            立即重启
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 重启确认对话框样式 */
.restart-dialog-overlay {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  backdrop-filter: blur(4px);
}

.restart-dialog {
  background: white;
  border-radius: 16px;
  width: 90%;
  max-width: 480px;
  max-height: 80vh;
  overflow-y: auto;
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3);
  animation: dialogSlideIn 0.3s ease-out;
}

@keyframes dialogSlideIn {
  from {
    opacity: 0;
    transform: translateY(-20px) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

.restart-dialog-header {
  padding: 24px 24px 0;
  text-align: center;
}

.restart-dialog-title {
  font-size: 20px;
  font-weight: 600;
  color: #1d1d1f;
  margin: 0;
}

.restart-dialog-body {
  padding: 24px;
  text-align: center;
}

.restart-icon {
  margin-bottom: 16px;
  color: #007aff;
}

.restart-message {
  font-size: 16px;
  color: #1d1d1f;
  margin: 0 0 16px;
  font-weight: 500;
}

.restart-description {
  font-size: 14px;
  color: #86868b;
  margin: 16px 0 0;
  line-height: 1.5;
}

.downloaded-files {
  background: #f2f2f7;
  border-radius: 12px;
  padding: 16px;
  margin: 16px 0;
  text-align: left;
}

.files-title {
  font-size: 13px;
  font-weight: 600;
  color: #1d1d1f;
  margin: 0 0 8px;
}

.files-list {
  list-style: none;
  padding: 0;
  margin: 0;
}

.file-item {
  padding: 4px 0;
  font-size: 13px;
  color: #86868b;
  border-bottom: 1px solid rgba(0, 0, 0, 0.05);
}

.file-item:last-child {
  border-bottom: none;
}

.restart-dialog-footer {
  padding: 0 24px 24px;
  display: flex;
  gap: 12px;
  justify-content: center;
}

.restart-btn {
  flex: 1;
  padding: 12px 24px;
  border: none;
  border-radius: 12px;
  font-size: 16px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
  min-height: 48px;
}

.restart-btn-cancel {
  background: #f2f2f7;
  color: #1d1d1f;
}

.restart-btn-cancel:hover {
  background: #e5e5ea;
}

.restart-btn-confirm {
  background: #007aff;
  color: white;
}

.restart-btn-confirm:hover {
  background: #0056cc;
}

.restart-btn:active {
  transform: translateY(1px);
}
</style>

<style>
@import './styles/settings.css';
@import './styles/components.css';
@import './styles/tags.css';
</style>
