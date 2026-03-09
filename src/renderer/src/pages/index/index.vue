<script setup lang="ts">
import IconSettings from '~icons/mdi/settings'
import IconKeyboard from '~icons/mdi/keyboard'
import IconInformation from '~icons/mdi/information'
import IconTag from '~icons/mdi/tag'
import IconBug from '~icons/mdi/bug'
import IconSync from '~icons/mdi/sync'
import IconOCR from '~icons/mdi/text-recognition'
import { useSettingsStore } from '@renderer/stores/common'

// 导入组件
import GeneralSettings from './components/GeneralSettings.vue'
import OCRSettings from './components/OCRSettings.vue'
import TagsManagement from './components/TagsManagement.vue'
import ShortcutSettings from './components/ShortcutSettings.vue'
import DebugSettings from './components/DebugSettings.vue'
import AboutSection from './components/AboutSection.vue'
import SyncSettings from './components/SyncSettings.vue'

// 导入API
import { onSimple } from '@renderer/api/ipc'
import { app } from '@renderer/api/renderer-api'
import { ref, computed, onMounted, onUnmounted, type Component } from 'vue'

type SectionId = 'general' | 'sync' | 'ocr' | 'tags' | 'shortcuts' | 'debug' | 'about'

interface SectionConfig {
  id: SectionId
  name: string
  icon: Component
}

interface SectionRendererConfig {
  component: Component
  getProps?: () => Record<string, unknown>
}

const activeSection = ref<SectionId>('general')
const settingsStore = useSettingsStore()

const showRestartDialog = ref(false)
const restartDialogData = ref<{
  downloadedFiles: string[]
  message: string
} | null>(null)

const RESTART_FAIL_MESSAGE = '重启应用失败，请手动重启应用以应用更改'

const settingSections: SectionConfig[] = [
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
    id: 'ocr',
    name: 'OCR 设置',
    icon: IconOCR
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

const sectionRendererMap: Record<SectionId, SectionRendererConfig> = {
  general: {
    component: GeneralSettings,
    getProps: () => ({ settingsStore })
  },
  sync: {
    component: SyncSettings
  },
  ocr: {
    component: OCRSettings,
    getProps: () => ({ settingsStore })
  },
  tags: {
    component: TagsManagement
  },
  shortcuts: {
    component: ShortcutSettings,
    getProps: () => ({ settingsStore })
  },
  debug: {
    component: DebugSettings,
    getProps: () => ({ settingsStore })
  },
  about: {
    component: AboutSection
  }
}

const currentSection = computed(() => {
  return settingSections.find((section) => section.id === activeSection.value) ?? settingSections[0]
})

const currentSectionName = computed(() => currentSection.value.name)
const currentSectionRenderer = computed(
  () => sectionRendererMap[activeSection.value] ?? sectionRendererMap.general
)
const currentSectionComponent = computed(() => currentSectionRenderer.value.component)
const currentSectionProps = computed(() => currentSectionRenderer.value.getProps?.() ?? {})

const setActiveSection = (sectionId: SectionId): void => {
  activeSection.value = sectionId
}

const runWindowAction = async (
  action: () => Promise<boolean>,
  actionName: string
): Promise<void> => {
  const success = await action()
  if (!success) {
    console.error(`${actionName}失败`)
  }
}

const minimizeWindow = async (): Promise<void> =>
  runWindowAction(() => app.minimizeWindow(), '最小化窗口')

const closeWindow = async (): Promise<void> => {
  await runWindowAction(() => app.closeWindow(), '关闭窗口')
}

const handleSyncRequiresRestart = (data: { downloadedFiles: string[]; message: string }): void => {
  restartDialogData.value = data
  showRestartDialog.value = true
}

const handleRestartFailure = (error?: unknown): void => {
  if (error) {
    console.error('重启应用失败:', error)
  }
  alert(RESTART_FAIL_MESSAGE)
}

const confirmRestart = async (): Promise<void> => {
  try {
    showRestartDialog.value = false
    const success = await app.restart()
    if (!success) {
      handleRestartFailure()
    }
  } catch (error) {
    handleRestartFailure(error)
  }
}

const cancelRestart = (): void => {
  showRestartDialog.value = false
  restartDialogData.value = null
}

let restartRequiredUnsubscribe: (() => void) | null = null

onMounted(() => {
  restartRequiredUnsubscribe = onSimple('sync-requires-restart', handleSyncRequiresRestart)
})

onUnmounted(() => {
  restartRequiredUnsubscribe?.()
  restartRequiredUnsubscribe = null
})
</script>

<template>
  <div class="settings-shell">
    <header class="custom-titlebar">
      <div class="titlebar-drag-area">
        <div class="titlebar-brand">
          <span class="titlebar-mark" />
          <span class="titlebar-app-name">Super Paste</span>
          <span class="titlebar-divider" />
          <span class="titlebar-page-name">{{ currentSectionName }}</span>
        </div>
      </div>

      <div class="window-controls">
        <button
          type="button"
          class="window-control-btn"
          aria-label="最小化窗口"
          @click="minimizeWindow"
        >
          <span class="window-control-icon minimize-icon" />
        </button>
        <button
          type="button"
          class="window-control-btn close-btn"
          aria-label="关闭窗口"
          @click="closeWindow"
        >
          <span class="window-control-icon close-icon" />
        </button>
      </div>
    </header>

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
        <component :is="currentSectionComponent" v-bind="currentSectionProps" />
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
            <button type="button" class="restart-btn restart-btn-cancel" @click="cancelRestart">
              稍后重启
            </button>
            <button type="button" class="restart-btn restart-btn-confirm" @click="confirmRestart">
              立即重启
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-shell {
  height: 100vh;
  width: 100vw;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.custom-titlebar {
  height: 52px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px 0 18px;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.92) 0%, rgba(244, 247, 251, 0.88) 100%);
  border-bottom: 1px solid rgba(15, 23, 42, 0.08);
  backdrop-filter: blur(14px);
  -webkit-app-region: drag;
}

.titlebar-drag-area {
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: center;
}

.titlebar-brand {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.titlebar-mark {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  background: linear-gradient(135deg, #0ea5e9 0%, #2563eb 100%);
  box-shadow: 0 0 0 2px rgba(37, 99, 235, 0.14);
}

.titlebar-app-name {
  font-size: 14px;
  color: #0f172a;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.titlebar-divider {
  width: 1px;
  height: 14px;
  background: rgba(100, 116, 139, 0.35);
}

.titlebar-page-name {
  font-size: 13px;
  color: #475569;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.window-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  -webkit-app-region: no-drag;
}

.window-control-btn {
  width: 30px;
  height: 30px;
  border-radius: 8px;
  border: none;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  background: rgba(15, 23, 42, 0.08);
  color: #1e293b;
  transition: all 0.2s ease;
}

.window-control-btn:hover {
  background: rgba(15, 23, 42, 0.14);
}

.window-control-btn:active {
  transform: translateY(1px);
}

.close-btn:hover {
  background: #ef4444;
  color: #ffffff;
}

.window-control-icon {
  display: block;
  position: relative;
}

.minimize-icon {
  width: 12px;
  height: 2px;
  border-radius: 999px;
  background: currentColor;
}

.close-icon {
  width: 12px;
  height: 12px;
}

.close-icon::before,
.close-icon::after {
  content: '';
  position: absolute;
  left: 5px;
  top: 0;
  width: 2px;
  height: 12px;
  border-radius: 999px;
  background: currentColor;
}

.close-icon::before {
  transform: rotate(45deg);
}

.close-icon::after {
  transform: rotate(-45deg);
}

.settings-container {
  flex: 1;
  height: calc(100vh - 52px);
  padding: var(--spacing-4);
  gap: var(--spacing-4);
}

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
