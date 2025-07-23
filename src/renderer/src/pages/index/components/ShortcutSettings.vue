<script setup lang="ts">
// Props
interface Props {
  settingsStore: any
}

const props = defineProps<Props>()

// 快捷键设置相关状态
const isListeningForShortcut = ref(false)
const pressedKeys = ref<Set<string>>(new Set())
const shortcutPreview = ref('')
const shortcutError = ref('')
const validShortcutDetected = ref(false) // 标记是否检测到有效快捷键

// 计算属性：用于显示的快捷键格式
const displayShortcut = computed(() => {
  if (isListeningForShortcut.value) {
    return shortcutPreview.value || '请按下快捷键...'
  }

  // 将存储的 Electron Accelerator 格式转换为用户友好的显示格式
  if (props.settingsStore.globalShortcut) {
    return convertElectronAcceleratorToDisplay(props.settingsStore.globalShortcut)
  }

  return '未设置快捷键' // 空状态显示
})

// 计算属性：检查是否有快捷键设置
const hasShortcut = computed(() => {
  return props.settingsStore.globalShortcut && props.settingsStore.globalShortcut.trim() !== ''
})

// Electron Accelerator 键盘按键映射
const electronKeyMap: Record<string, string> = {
  // 修饰键
  'Control': 'CommandOrControl',
  'Meta': 'CommandOrControl', // Mac 上的 Cmd 键，Windows/Linux 上的 Ctrl 键
  'Alt': 'Alt',
  'Shift': 'Shift',

  // 特殊键
  ' ': 'Space',
  'Enter': 'Return',
  'Escape': 'Escape',
  'Backspace': 'Backspace',
  'Delete': 'Delete',
  'Tab': 'Tab',
  'CapsLock': 'CapsLock',

  // 方向键
  'ArrowUp': 'Up',
  'ArrowDown': 'Down',
  'ArrowLeft': 'Left',
  'ArrowRight': 'Right',

  // 功能键
  'F1': 'F1', 'F2': 'F2', 'F3': 'F3', 'F4': 'F4',
  'F5': 'F5', 'F6': 'F6', 'F7': 'F7', 'F8': 'F8',
  'F9': 'F9', 'F10': 'F10', 'F11': 'F11', 'F12': 'F12',

  // 数字键盘
  'Insert': 'Insert',
  'Home': 'Home',
  'End': 'End',
  'PageUp': 'PageUp',
  'PageDown': 'PageDown',

  // 其他特殊键
  'PrintScreen': 'PrintScreen',
  'ScrollLock': 'ScrollLock',
  'Pause': 'Pause'
}

// 格式化按键名称为 Electron Accelerator 格式
const formatKeyForElectron = (key: string): string => {
  // 如果是映射表中的键，使用映射值
  if (electronKeyMap[key]) {
    return electronKeyMap[key]
  }

  // 对于字母和数字，直接使用大写
  if (/^[a-zA-Z0-9]$/.test(key)) {
    return key.toUpperCase()
  }

  // 对于符号键，需要特殊处理
  const symbolMap: Record<string, string> = {
    '`': 'grave',
    '-': 'minus',
    '=': 'equal',
    '[': 'bracketleft',
    ']': 'bracketright',
    '\\': 'backslash',
    ';': 'semicolon',
    "'": 'apostrophe',
    ',': 'comma',
    '.': 'period',
    '/': 'slash',
    '1': '1', '2': '2', '3': '3', '4': '4', '5': '5',
    '6': '6', '7': '7', '8': '8', '9': '9', '0': '0'
  }

  return symbolMap[key] || key.toUpperCase()
}

// 用于显示的格式化函数（用户友好的显示）
const formatKeyForDisplay = (key: string): string => {
  const displayMap: Record<string, string> = {
    'Control': 'Ctrl',
    'Meta': window.electron.process.platform === 'darwin' ? 'Cmd' : 'Ctrl',
    'Alt': 'Alt',
    'Shift': 'Shift',
    ' ': 'Space',
    'Enter': 'Enter',
    'Escape': 'Esc',
    'Backspace': 'Backspace',
    'Delete': 'Del',
    'Tab': 'Tab',
    'ArrowUp': '↑',
    'ArrowDown': '↓',
    'ArrowLeft': '←',
    'ArrowRight': '→'
  }

  return displayMap[key] || key.toUpperCase()
}

// 将 Electron Accelerator 格式转换为用户友好的显示格式
const convertElectronAcceleratorToDisplay = (accelerator: string): string => {
  if (!accelerator) return ''

  const electronToDisplayMap: Record<string, string> = {
    'CommandOrControl': window.electron.process.platform === 'darwin' ? 'Cmd' : 'Ctrl',
    'Control': 'Ctrl',
    'Meta': window.electron.process.platform === 'darwin' ? 'Cmd' : 'Ctrl',
    'Alt': 'Alt',
    'Shift': 'Shift',
    'Space': 'Space',
    'Return': 'Enter',
    'Escape': 'Esc',
    'Backspace': 'Backspace',
    'Delete': 'Del',
    'Tab': 'Tab',
    'Up': '↑',
    'Down': '↓',
    'Left': '←',
    'Right': '→'
  }

  return accelerator
    .split('+')
    .map(key => electronToDisplayMap[key] || key)
    .join('+')
}

// 设置全局快捷键（存储 Electron Accelerator 格式）
const setGlobalShortcut = (electronAccelerator: string): void => {
  // 存储 Electron Accelerator 格式，用于实际的快捷键注册
  props.settingsStore.globalShortcut = electronAccelerator
  console.log('全局快捷键已保存 (Electron格式):', electronAccelerator)
  console.log('显示格式:', convertElectronAcceleratorToDisplay(electronAccelerator))
}

// 保存检测到的有效快捷键
const saveDetectedShortcut = (): void => {
  // 生成 Electron Accelerator 格式的快捷键（用于存储和实际注册）
  const electronAccelerator = Array.from(pressedKeys.value)
    .sort((a, b) => {
      // 修饰键排序：Control, Meta, Alt, Shift, 其他键
      const order = ['Control', 'Meta', 'Alt', 'Shift']
      const aIndex = order.indexOf(a)
      const bIndex = order.indexOf(b)
      if (aIndex !== -1 && bIndex !== -1) return aIndex - bIndex
      if (aIndex !== -1) return -1
      if (bIndex !== -1) return 1
      return a.localeCompare(b)
    })
    .map(formatKeyForElectron)
    .join('+')

  // 生成用户友好的显示格式
  const displayFormat = Array.from(pressedKeys.value)
    .sort((a, b) => {
      const order = ['Control', 'Meta', 'Alt', 'Shift']
      const aIndex = order.indexOf(a)
      const bIndex = order.indexOf(b)
      if (aIndex !== -1 && bIndex !== -1) return aIndex - bIndex
      if (aIndex !== -1) return -1
      if (bIndex !== -1) return 1
      return a.localeCompare(b)
    })
    .map(formatKeyForDisplay)
    .join('+')

  console.log('快捷键设置成功:')
  console.log('  - 存储格式 (Electron Accelerator):', electronAccelerator)
  console.log('  - 显示格式 (用户友好):', displayFormat)

  // 保存到存储（Electron Accelerator 格式）
  setGlobalShortcut(electronAccelerator)
  stopListeningForShortcut()
}

// 开始监听快捷键
const startListeningForShortcut = () => {
  isListeningForShortcut.value = true
  pressedKeys.value.clear()
  shortcutPreview.value = ''
  shortcutError.value = ''

  console.log('开始监听快捷键输入...')

  // 添加键盘事件监听
  document.addEventListener('keydown', handleKeyDown)
  document.addEventListener('keyup', handleKeyUp)
}

// 停止监听快捷键
const stopListeningForShortcut = (): void => {
  isListeningForShortcut.value = false
  pressedKeys.value.clear()
  shortcutPreview.value = ''
  shortcutError.value = ''
  validShortcutDetected.value = false

  console.log('停止监听快捷键输入')

  // 移除键盘事件监听
  document.removeEventListener('keydown', handleKeyDown)
  document.removeEventListener('keyup', handleKeyUp)
}

// 处理按键按下
const handleKeyDown = (event: KeyboardEvent): void => {
  event.preventDefault()
  event.stopPropagation()

  if (!isListeningForShortcut.value) return

  // 标准化按键名称（将字母键转为小写，保持修饰键原样）
  let normalizedKey = event.key
  if (event.key.length === 1 && /[A-Z]/.test(event.key)) {
    normalizedKey = event.key.toLowerCase()
  }

  // 添加按键到集合
  pressedKeys.value.add(normalizedKey)

  console.log('按键按下:', event.key, '标准化后:', normalizedKey, '当前按键集合:', Array.from(pressedKeys.value))

  // 更新预览
  updateShortcutPreview()
}

// 验证快捷键是否有效
const validateShortcut = (keys: Set<string>): { valid: boolean; error: string } => {
  const keyArray = Array.from(keys)
  console.log('验证快捷键，按键数组:', keyArray)

  // 检查键数量：最少2个，最多3个
  if (keyArray.length < 2) {
    console.log('验证失败：键数量不足，当前数量:', keyArray.length)
    return { valid: false, error: '至少需要2个键' }
  }
  if (keyArray.length > 3) {
    console.log('验证失败：键数量过多，当前数量:', keyArray.length)
    return { valid: false, error: '最多只能3个键' }
  }

  // 检查是否包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）
  const modifierKeys = ['Control', 'Meta', 'Alt', 'Shift']
  const hasModifier = keyArray.some(key => modifierKeys.includes(key))
  console.log('修饰键检查，找到的修饰键:', keyArray.filter(key => modifierKeys.includes(key)))

  if (!hasModifier) {
    console.log('验证失败：缺少修饰键')
    return { valid: false, error: '必须包含至少一个修饰键（Ctrl、Cmd、Alt、Shift）' }
  }

  // 检查是否至少有一个非修饰键
  const nonModifierKeys = keyArray.filter(key => !modifierKeys.includes(key))
  console.log('非修饰键:', nonModifierKeys)

  if (nonModifierKeys.length === 0) {
    console.log('验证失败：缺少非修饰键')
    return { valid: false, error: '需要至少一个字母或数字键' }
  }

  console.log('验证成功：快捷键有效')
  return { valid: true, error: '' }
}

// 处理按键释放
const handleKeyUp = (event: KeyboardEvent): void => {
  event.preventDefault()
  event.stopPropagation()

  if (!isListeningForShortcut.value) return

  console.log('按键释放:', event.key, '当前按键集合:', Array.from(pressedKeys.value), '有效快捷键已检测:', validShortcutDetected.value)

  // 如果已经检测到有效快捷键，直接保存
  if (validShortcutDetected.value) {
    console.log('检测到有效快捷键，立即保存')
    saveDetectedShortcut()
    return
  }

  // 如果释放的是修饰键，继续监听
  if (['Control', 'Meta', 'Alt', 'Shift'].includes(event.key)) {
    console.log('释放的是修饰键，继续监听')
    return
  }

  // 使用短暂延迟确保所有按键都已记录
  setTimeout(() => {
    console.log('延迟后开始验证快捷键，当前按键集合:', Array.from(pressedKeys.value))

    // 验证快捷键是否有效
    const validation = validateShortcut(pressedKeys.value)
    console.log('验证结果:', validation)

    if (validation.valid) {
      // 生成 Electron Accelerator 格式的快捷键（用于存储和实际注册）
      const electronAccelerator = Array.from(pressedKeys.value)
        .sort((a, b) => {
          // 修饰键排序：Control, Meta, Alt, Shift, 其他键
          const order = ['Control', 'Meta', 'Alt', 'Shift']
          const aIndex = order.indexOf(a)
          const bIndex = order.indexOf(b)
          if (aIndex !== -1 && bIndex !== -1) return aIndex - bIndex
          if (aIndex !== -1) return -1
          if (bIndex !== -1) return 1
          return a.localeCompare(b)
        })
        .map(formatKeyForElectron)
        .join('+')

      // 生成用户友好的显示格式
      const displayFormat = Array.from(pressedKeys.value)
        .sort((a, b) => {
          const order = ['Control', 'Meta', 'Alt', 'Shift']
          const aIndex = order.indexOf(a)
          const bIndex = order.indexOf(b)
          if (aIndex !== -1 && bIndex !== -1) return aIndex - bIndex
          if (aIndex !== -1) return -1
          if (bIndex !== -1) return 1
          return a.localeCompare(b)
        })
        .map(formatKeyForDisplay)
        .join('+')

      console.log('快捷键设置成功:')
      console.log('  - 存储格式 (Electron Accelerator):', electronAccelerator)
      console.log('  - 显示格式 (用户友好):', displayFormat)

      // 保存到存储（Electron Accelerator 格式）
      setGlobalShortcut(electronAccelerator)
      stopListeningForShortcut()
    } else {
      // 显示错误信息并清除当前按键
      shortcutError.value = validation.error
      pressedKeys.value.clear()
      shortcutPreview.value = ''

      console.log('快捷键验证失败:', validation.error)

      // 3秒后清除错误信息
      setTimeout(() => {
        shortcutError.value = ''
      }, 3000)
    }
  }, 50) // 50ms 延迟确保所有按键事件都已处理
}

// 更新快捷键预览（显示用户友好的格式）
const updateShortcutPreview = (): void => {
  if (pressedKeys.value.size === 0) {
    shortcutPreview.value = ''
    validShortcutDetected.value = false
    return
  }

  // 生成用于显示的快捷键字符串（用户友好格式）
  const displayKeys = Array.from(pressedKeys.value)
    .sort((a, b) => {
      // 修饰键排序：Control, Meta, Alt, Shift, 其他键
      const order = ['Control', 'Meta', 'Alt', 'Shift']
      const aIndex = order.indexOf(a)
      const bIndex = order.indexOf(b)
      if (aIndex !== -1 && bIndex !== -1) return aIndex - bIndex
      if (aIndex !== -1) return -1
      if (bIndex !== -1) return 1
      return a.localeCompare(b)
    })
    .map(formatKeyForDisplay)
    .join('+')

  // 添加验证状态提示
  const validation = validateShortcut(pressedKeys.value)
  if (!validation.valid) {
    shortcutPreview.value = `${displayKeys} (${validation.error})`
    validShortcutDetected.value = false
  } else {
    shortcutPreview.value = displayKeys
    validShortcutDetected.value = true
    console.log('检测到有效快捷键，标记为可保存')
  }

  console.log('快捷键预览更新:', shortcutPreview.value)
}

// 取消设置快捷键
const cancelShortcutSetting = () => {
  stopListeningForShortcut()
}

// 清除快捷键
const clearShortcut = () => {
  if (confirm('确定要清除当前快捷键设置吗？')) {
    const oldShortcut = props.settingsStore.globalShortcut
    props.settingsStore.globalShortcut = ''
    console.log('快捷键已清除:', convertElectronAcceleratorToDisplay(oldShortcut), '→ 未设置')
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
      <!-- 全局快捷键 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">启动覆盖层快捷键</h4>
          <p class="setting-description">设置打开覆盖层窗口的全局快捷键</p>
        </div>
        <div class="setting-control">
          <div class="shortcut-input">
            <div class="shortcut-display">
              <input type="text" :value="displayShortcut" readonly
                :placeholder="isListeningForShortcut ? '请按下快捷键组合...' : '点击设置快捷键'"
                :class="['shortcut-field', { 'listening': isListeningForShortcut, 'preview': shortcutPreview, 'empty': !hasShortcut }]">
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
