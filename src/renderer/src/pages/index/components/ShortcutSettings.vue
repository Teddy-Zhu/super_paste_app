<script setup lang="ts">
import { useSettingsStore } from '@renderer/stores/common'
import { ref, computed, onMounted, onUnmounted } from 'vue'
import {
  getSystemAccessStatus,
  SYSTEM_ACCESS_TEXT,
  type SystemAccessStatus
} from '@renderer/api/system-access'

const accessStatus = ref<SystemAccessStatus>()
const accessError = ref('')
const accessBusy = ref(false)
const refreshAccess = async (retry = false): Promise<void> => {
  accessBusy.value = true
  accessError.value = ''
  try {
    accessStatus.value = await getSystemAccessStatus(retry)
  } catch (error) {
    accessError.value = error instanceof Error ? error.message : String(error)
  } finally {
    accessBusy.value = false
  }
}
onMounted(() => refreshAccess())

const settingsStore = useSettingsStore()

// 快捷键设置相关状态
const isListeningForShortcut = ref(false)
const shortcutPreview = ref('')
const shortcutError = ref('')
const validShortcutDetected = ref(false)
const detectedShortcut = ref('')

// 计算属性：用于显示的快捷键格式
const displayShortcut = computed(() => {
  if (isListeningForShortcut.value) {
    return shortcutPreview.value || '请按下快捷键...'
  }

  // 将存储的 Electron Accelerator 格式转换为用户友好的显示格式
  if (settingsStore.globalShortcut) {
    return convertElectronAcceleratorToDisplay(settingsStore.globalShortcut)
  }

  return '未设置快捷键' // 空状态显示
})

// 计算属性：检查是否有快捷键设置
const hasShortcut = computed(() => {
  return settingsStore.globalShortcut && settingsStore.globalShortcut.trim() !== ''
})

// 与 shortcut-config.tsx 保持一致的按键映射
const shortcutKeyMap: Record<string, string> = {
  Backquote: '`',
  Backslash: '\\',
  BracketLeft: '[',
  BracketRight: ']',
  Comma: ',',
  Equal: '=',
  Minus: '-',
  Plus: 'PLUS',
  Period: '.',
  Quote: "'",
  Semicolon: ';',
  Slash: '/',
  Backspace: 'Backspace',
  CapsLock: 'Capslock',
  ContextMenu: 'Contextmenu',
  Space: 'Space',
  Tab: 'Tab',
  Convert: 'Convert',
  Delete: 'Delete',
  End: 'End',
  Help: 'Help',
  Home: 'Home',
  PageDown: 'Pagedown',
  PageUp: 'Pageup',
  Escape: 'Esc',
  PrintScreen: 'Printscreen',
  ScrollLock: 'Scrolllock',
  Pause: 'Pause',
  Insert: 'Insert',
  Suspend: 'Suspend'
}

const modifierEventKeys = ['Control', 'Meta', 'Alt', 'Shift']

const parseShortcutFromEvent = (event: KeyboardEvent): string => {
  let code = event.code
  const key = event.key

  if (code === 'Backspace') {
    return ''
  }

  let newValue = ''
  if (event.ctrlKey) {
    newValue = 'Ctrl'
  }
  if (event.shiftKey) {
    newValue = `${newValue}${newValue.length > 0 ? '+' : ''}Shift`
  }
  if (event.metaKey) {
    newValue = `${newValue}${newValue.length > 0 ? '+' : ''}${window.electron.process.platform === 'darwin' ? 'Command' : 'Super'}`
  }
  if (event.altKey) {
    newValue = `${newValue}${newValue.length > 0 ? '+' : ''}Alt`
  }

  if (code.startsWith('Key')) {
    code = code.substring(3)
  } else if (code.startsWith('Digit')) {
    code = code.substring(5)
  } else if (code.startsWith('Arrow')) {
    code = code.substring(5)
  } else if (key.startsWith('Arrow')) {
    code = key.substring(5)
  } else if (code.startsWith('Intl')) {
    code = code.substring(4)
  } else if (code.startsWith('Numpad')) {
    if (key.length === 1) {
      code = `Num${code.substring(6)}`
    } else {
      code = key
    }
  } else if (/F\d+/.test(code)) {
    // F1-F12 原样保留
  } else if (shortcutKeyMap[code] !== undefined) {
    code = shortcutKeyMap[code]
  } else {
    code = ''
  }

  return `${newValue}${newValue.length > 0 && code.length > 0 ? '+' : ''}${code}`
}

// 将 Electron Accelerator 格式转换为用户友好的显示格式
const convertElectronAcceleratorToDisplay = (accelerator: string): string => {
  if (!accelerator) return ''

  const electronToDisplayMap: Record<string, string> = {
    CommandOrControl: window.electron.process.platform === 'darwin' ? 'Cmd' : 'Ctrl',
    Ctrl: 'Ctrl',
    Command: 'Cmd',
    Super: 'Win',
    Control: 'Ctrl',
    Meta: window.electron.process.platform === 'darwin' ? 'Cmd' : 'Ctrl',
    Alt: 'Alt',
    Shift: 'Shift',
    Space: 'Space',
    Return: 'Enter',
    Escape: 'Esc',
    Backspace: 'Backspace',
    Delete: 'Del',
    Tab: 'Tab',
    Up: '↑',
    Down: '↓',
    Left: '←',
    Right: '→'
  }

  return accelerator
    .split('+')
    .map((key) => electronToDisplayMap[key] || key)
    .join('+')
}

// 设置全局快捷键（存储 Electron Accelerator 格式）
const setGlobalShortcut = async (electronAccelerator: string): Promise<boolean> => {
  try {
    settingsStore.globalShortcut = electronAccelerator
    await settingsStore.$whenSynced()
    await refreshAccess()
    return true
  } catch (error) {
    shortcutError.value = error instanceof Error ? error.message : String(error)
    return false
  }
}

// 开始监听快捷键
const startListeningForShortcut = (): void => {
  isListeningForShortcut.value = true
  shortcutPreview.value = ''
  shortcutError.value = ''
  validShortcutDetected.value = false
  detectedShortcut.value = ''

  // 添加键盘事件监听
  document.addEventListener('keydown', handleKeyDown)
  document.addEventListener('keyup', handleKeyUp)
}

// 停止监听快捷键
const stopListeningForShortcut = (): void => {
  isListeningForShortcut.value = false
  shortcutPreview.value = ''
  shortcutError.value = ''
  validShortcutDetected.value = false
  detectedShortcut.value = ''

  // 移除键盘事件监听
  document.removeEventListener('keydown', handleKeyDown)
  document.removeEventListener('keyup', handleKeyUp)
}

// 验证快捷键是否有效
const validateShortcut = (shortcut: string): { valid: boolean; error: string } => {
  const keyArray = shortcut
    .split('+')
    .map((key) => key.trim())
    .filter(Boolean)

  // 检查键数量：最少2个，最多3个
  if (keyArray.length < 2) {
    return { valid: false, error: '至少需要2个键' }
  }
  if (keyArray.length > 3) {
    return { valid: false, error: '最多只能3个键' }
  }

  // 检查是否包含至少一个修饰键
  const modifierKeys = ['Ctrl', 'Command', 'Super', 'Alt', 'Shift']
  const hasModifier = keyArray.some((key) => modifierKeys.includes(key))
  if (!hasModifier) {
    return { valid: false, error: '必须包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）' }
  }

  // 检查是否至少有一个非修饰键
  const nonModifierKeys = keyArray.filter((key) => !modifierKeys.includes(key))
  if (nonModifierKeys.length === 0) {
    return { valid: false, error: '需要至少一个字母或数字键' }
  }

  return { valid: true, error: '' }
}

const updatePreview = (shortcut: string): void => {
  if (!shortcut) {
    shortcutPreview.value = ''
    validShortcutDetected.value = false
    return
  }

  const displayValue = convertElectronAcceleratorToDisplay(shortcut)
  const validation = validateShortcut(shortcut)

  if (!validation.valid) {
    shortcutPreview.value = `${displayValue} (${validation.error})`
    validShortcutDetected.value = false
  } else {
    shortcutPreview.value = displayValue
    validShortcutDetected.value = true
  }
}

// 处理按键按下
const handleKeyDown = (event: KeyboardEvent): void => {
  event.preventDefault()
  event.stopPropagation()

  if (!isListeningForShortcut.value) return

  const shortcut = parseShortcutFromEvent(event)

  if (event.code === 'Backspace') {
    detectedShortcut.value = ''
    shortcutError.value = ''
    updatePreview('')
    return
  }

  if (!shortcut) {
    return
  }

  detectedShortcut.value = shortcut
  shortcutError.value = ''
  updatePreview(shortcut)
}

// 处理按键释放
const handleKeyUp = async (event: KeyboardEvent): Promise<void> => {
  event.preventDefault()
  event.stopPropagation()

  if (!isListeningForShortcut.value) return

  // 如果释放的是修饰键，继续监听
  if (modifierEventKeys.includes(event.key)) {
    return
  }

  if (!detectedShortcut.value) {
    return
  }

  const validation = validateShortcut(detectedShortcut.value)
  if (validation.valid) {
    if (await setGlobalShortcut(detectedShortcut.value)) stopListeningForShortcut()
    return
  }

  shortcutError.value = validation.error
  detectedShortcut.value = ''
  updatePreview('')

  // 3秒后清除错误信息
  setTimeout(() => {
    shortcutError.value = ''
  }, 3000)
}

// 取消设置快捷键
const cancelShortcutSetting = (): void => {
  stopListeningForShortcut()
}

// 清除快捷键
const clearShortcut = async (): Promise<void> => {
  if (confirm('确定要清除当前快捷键设置吗？')) {
    await setGlobalShortcut('')
  }
}

// 组件卸载时清理事件监听
onUnmounted(() => {
  if (isListeningForShortcut.value) {
    stopListeningForShortcut()
  }
})
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">快捷键设置</h3>
      <p class="section-subtitle">自定义全局快捷键</p>
    </div>

    <div class="settings-grid">
      <div v-if="accessStatus || accessError" class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">{{ SYSTEM_ACCESS_TEXT.title }}</h4>
          <p v-if="accessError" class="shortcut-error">{{ accessError }}</p>
          <template v-if="accessStatus">
            <p class="setting-description">
              {{
                accessStatus.shortcut.error ||
                SYSTEM_ACCESS_TEXT[
                  accessStatus.shortcut.state === 'portal-requested'
                    ? 'portal'
                    : accessStatus.shortcut.state === 'registered'
                      ? 'registered'
                      : 'disabled'
                ]
              }}
            </p>
            <p class="setting-description">
              {{
                accessStatus.clipboard.error ||
                (accessStatus.clipboard.listening
                  ? SYSTEM_ACCESS_TEXT.clipboardReady
                  : SYSTEM_ACCESS_TEXT.clipboardStopped)
              }}
            </p>
            <p class="setting-description">
              {{
                accessStatus.sourceTracking
                  ? SYSTEM_ACCESS_TEXT.sourceReady
                  : SYSTEM_ACCESS_TEXT.sourceUnavailable
              }}
            </p>
            <template v-if="accessStatus.wayland">
              <p class="setting-description">{{ SYSTEM_ACCESS_TEXT.wayland }}</p>
              <p class="setting-description">{{ SYSTEM_ACCESS_TEXT.fallback }}</p>
              <code class="system-shortcut-command"
                >"{{ accessStatus.executable }}" --toggle-overlay</code
              >
            </template>
          </template>
        </div>
        <div class="setting-control shortcut-buttons">
          <button class="shortcut-button secondary" :disabled="accessBusy" @click="refreshAccess()">
            {{ SYSTEM_ACCESS_TEXT.refresh }}
          </button>
          <button
            class="shortcut-button primary"
            :disabled="accessBusy"
            @click="refreshAccess(true)"
          >
            {{ SYSTEM_ACCESS_TEXT.retry }}
          </button>
        </div>
      </div>
      <!-- 全局快捷键 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">启动覆盖层快捷键</h4>
          <p class="setting-description">设置打开覆盖层窗口的全局快捷键</p>
        </div>
        <div class="setting-control">
          <div class="shortcut-input">
            <div class="shortcut-display">
              <input
                type="text"
                :value="displayShortcut"
                readonly
                :placeholder="isListeningForShortcut ? '请按下快捷键组合...' : '点击设置快捷键'"
                :class="[
                  'shortcut-field',
                  {
                    listening: isListeningForShortcut,
                    preview: shortcutPreview,
                    empty: !hasShortcut
                  }
                ]"
              />
            </div>
            <div class="shortcut-buttons">
              <template v-if="!isListeningForShortcut">
                <button class="shortcut-button primary" @click="startListeningForShortcut">
                  {{ hasShortcut ? '重新设置' : '设置快捷键' }}
                </button>
                <button v-if="hasShortcut" class="shortcut-button danger" @click="clearShortcut">
                  清除
                </button>
              </template>
              <template v-else>
                <button class="shortcut-button secondary" @click="cancelShortcutSetting">
                  取消
                </button>
                <span class="shortcut-hint">按下快捷键组合，释放最后一个键完成设置</span>
              </template>
            </div>
            <!-- 错误提示 -->
            <div v-if="shortcutError" class="shortcut-error">
              {{ shortcutError }}
            </div>
            <!-- 快捷键要求说明 -->
            <div class="shortcut-requirements">
              <p class="requirement-text">快捷键要求：</p>
              <ul class="requirement-list">
                <li>最少2个键，最多3个键</li>
                <li>必须包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）</li>
                <li>至少包含一个字母或数字键</li>
              </ul>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.system-shortcut-command {
  display: block;
  overflow-wrap: anywhere;
  user-select: text;
}
</style>
