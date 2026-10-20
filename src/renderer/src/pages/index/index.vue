<script setup lang="ts">
import IconSettings from '~icons/mdi/settings'
import IconKeyboard from '~icons/mdi/keyboard'
import IconInformation from '~icons/mdi/information'
import IconTag from '~icons/mdi/tag'
import IconBug from '~icons/mdi/bug'
import IconSync from '~icons/mdi/sync'
import IconOCR from '~icons/mdi/text-recognition'
import IconClipboard from '~icons/mdi/content-paste'
import { SETTINGS_LABELS } from '@renderer/constants/desktop-ui'
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
import { ref, computed, watch, onMounted, onUnmounted, type Component } from 'vue'

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
const isWindowFocused = ref(document.hasFocus())
const restartDialog = ref<HTMLDialogElement>()

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

const handleNavigationKeydown = (event: KeyboardEvent): void => {
  if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return
  const navigation = event.currentTarget as HTMLElement
  const buttons = Array.from(navigation.querySelectorAll<HTMLButtonElement>('.nav-item'))
  const currentIndex = buttons.indexOf(document.activeElement as HTMLButtonElement)
  if (currentIndex < 0) return
  event.preventDefault()
  let nextIndex = currentIndex
  if (event.key === 'Home') nextIndex = 0
  else if (event.key === 'End') nextIndex = buttons.length - 1
  else
    nextIndex =
      (currentIndex + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length
  setActiveSection(settingSections[nextIndex].id)
  buttons[nextIndex].focus()
}

const handleWindowFocus = (): void => {
  isWindowFocused.value = true
}
const handleWindowBlur = (): void => {
  isWindowFocused.value = false
}

watch(
  showRestartDialog,
  (visible) => {
    if (visible) restartDialog.value?.showModal()
  },
  { flush: 'post' }
)

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
  window.addEventListener('focus', handleWindowFocus)
  window.addEventListener('blur', handleWindowBlur)
})

onUnmounted(() => {
  window.removeEventListener('focus', handleWindowFocus)
  window.removeEventListener('blur', handleWindowBlur)
  restartDialog.value?.close()
  restartRequiredUnsubscribe?.()
  restartRequiredUnsubscribe = null
})
</script>

<template>
  <div class="settings-shell" :class="{ 'settings-shell--inactive': !isWindowFocused }">
    <header class="custom-titlebar">
      <div class="titlebar-drag-area">
        <div class="titlebar-brand">
          <IconClipboard class="titlebar-mark" />
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
          :title="SETTINGS_LABELS.minimize"
          @click="minimizeWindow"
        >
          <span class="window-control-icon minimize-icon" />
        </button>
        <button
          type="button"
          class="window-control-btn close-btn"
          aria-label="关闭窗口"
          :title="SETTINGS_LABELS.close"
          @click="closeWindow"
        >
          <span class="window-control-icon close-icon" />
        </button>
      </div>
    </header>

    <div class="settings-container">
      <!-- 左侧边栏 -->
      <aside class="sidebar">
        <div class="sidebar-header">
          <p class="app-subtitle">{{ SETTINGS_LABELS.preferences }}</p>
        </div>

        <nav
          class="sidebar-nav"
          :aria-label="SETTINGS_LABELS.navigation"
          @keydown="handleNavigationKeydown"
        >
          <button
            v-for="section in settingSections"
            :key="section.id"
            type="button"
            class="nav-item"
            :class="{
              active: activeSection === section.id,
              'nav-item--utility': section.id === 'debug'
            }"
            :aria-current="activeSection === section.id ? 'page' : undefined"
            @click="setActiveSection(section.id)"
          >
            <component :is="section.icon" class="nav-icon" />
            <span class="nav-text">{{ section.name }}</span>
          </button>
        </nav>
      </aside>

      <!-- 右侧内容区域 -->
      <main class="main-content">
        <Transition name="settings-page" mode="out-in">
          <component
            :is="currentSectionComponent"
            :key="activeSection"
            v-bind="currentSectionProps"
          />
        </Transition>
      </main>

      <!-- 重启确认对话框 -->
      <dialog
        v-if="showRestartDialog"
        ref="restartDialog"
        class="restart-dialog"
        aria-labelledby="restart-dialog-title"
        @cancel.prevent="cancelRestart"
        @click="$event.target === $event.currentTarget && cancelRestart()"
      >
        <div class="restart-dialog-header">
          <h3 id="restart-dialog-title" class="restart-dialog-title">同步完成</h3>
        </div>

        <div class="restart-dialog-body">
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
      </dialog>
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
  border: 1px solid var(--color-border-strong);
  background: var(--color-window);
}

.custom-titlebar {
  height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-left: 14px;
  background: var(--color-titlebar);
  border-bottom: 1px solid var(--color-border-medium);
  -webkit-app-region: drag;
}

.titlebar-drag-area {
  min-width: 0;
  flex: 1;
}

.titlebar-brand {
  display: flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
}

.titlebar-mark {
  width: 16px;
  height: 16px;
  color: var(--color-primary);
  flex-shrink: 0;
}

.titlebar-app-name {
  font-size: 12px;
  font-weight: 600;
}

.titlebar-divider {
  width: 1px;
  height: 12px;
  background: var(--color-border-strong);
}

.titlebar-page-name {
  font-size: 12px;
  color: var(--color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.settings-shell--inactive .custom-titlebar {
  color: var(--color-text-tertiary);
  background: var(--color-window);
}

.window-controls {
  display: flex;
  align-self: stretch;
  -webkit-app-region: no-drag;
}

.window-control-btn {
  width: 44px;
  padding: 0;
  border: none;
  display: grid;
  place-items: center;
  cursor: pointer;
  background: transparent;
  color: var(--color-text-secondary);
  transition:
    background-color var(--duration-fast),
    color var(--duration-fast);
}

.window-control-btn:hover {
  background: var(--color-control-hover);
  color: var(--color-text-primary);
}

.window-control-btn:active {
  background: var(--color-control-active);
}

.window-control-btn:focus-visible {
  outline-offset: -3px;
}

.close-btn:hover {
  background: #c94040;
  color: white;
}

.close-btn:active {
  background: #ad3434;
}

.window-control-icon {
  display: block;
  position: relative;
}

.minimize-icon {
  width: 10px;
  height: 1px;
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
  width: 1px;
  height: 12px;
  background: currentColor;
}

.close-icon::before {
  transform: rotate(45deg);
}

.close-icon::after {
  transform: rotate(-45deg);
}

.restart-dialog {
  width: min(420px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  margin: auto;
  padding: 0;
  overflow-y: auto;
  border: 1px solid var(--color-border-strong);
  border-radius: var(--radius-lg);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-soft-xl);
}

.restart-dialog[open] {
  animation: surface-appear var(--duration-normal) var(--ease-out);
}

.restart-dialog::backdrop {
  background: rgba(28, 38, 54, 0.22);
}

.restart-dialog-header {
  padding: 16px 20px;
  background: var(--color-bg-secondary);
  border-bottom: 1px solid var(--color-border-light);
}

.restart-dialog-title {
  font-size: 14px;
  font-weight: 600;
  margin: 0;
}

.restart-dialog-body {
  padding: 20px;
}

.restart-message {
  font-size: 13px;
  margin: 0;
  line-height: 1.5;
}

.restart-description {
  font-size: 12px;
  color: var(--color-text-secondary);
  margin: 12px 0 0;
  line-height: 1.6;
}

.downloaded-files {
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-xs);
  padding: 12px;
  margin-top: 16px;
}

.files-title {
  font-size: 12px;
  font-weight: 500;
  margin: 0 0 6px;
}

.files-list {
  list-style: none;
  padding: 0;
  margin: 0;
}

.file-item {
  padding: 3px 0;
  font-size: 12px;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}

.restart-dialog-footer {
  padding: 12px 20px;
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  background: var(--color-bg-secondary);
  border-top: 1px solid var(--color-border-light);
}

.restart-btn {
  min-width: 84px;
  min-height: 28px;
  padding: 4px 12px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  box-shadow: var(--shadow-control);
}

.restart-btn-cancel {
  background: var(--color-bg-primary);
}

.restart-btn-cancel:hover {
  background: var(--color-control-hover);
}

.restart-btn-confirm {
  background: var(--color-primary);
  border-color: var(--color-primary-hover);
  color: white;
}

.restart-btn-confirm:hover {
  background: var(--color-primary-hover);
}
</style>

<style>
@import './styles/settings.css';
@import './styles/components.css';
@import './styles/tags.css';
</style>
