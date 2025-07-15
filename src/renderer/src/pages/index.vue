<script setup lang="ts">
import { send } from '@renderer/api'
import { tags, clipboard } from '@renderer/api/renderer-api'
import type { TagGroup } from '@renderer/api/types'
import IconSettings from '~icons/mdi/settings'
import IconKeyboard from '~icons/mdi/keyboard'
import IconInformation from '~icons/mdi/information'
import IconTag from '~icons/mdi/tag'
import IconBug from '~icons/mdi/bug'

// 当前选中的设置项
const activeSection = ref('general')
const settingsStore = useSettingsStore()

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
  if (settingsStore.globalShortcut) {
    return convertElectronAcceleratorToDisplay(settingsStore.globalShortcut)
  }

  return '未设置快捷键' // 空状态显示
})

// 计算属性：检查是否有快捷键设置
const hasShortcut = computed(() => {
  return settingsStore.globalShortcut && settingsStore.globalShortcut.trim() !== ''
})

// 设置项列表
const settingSections = [
  {
    id: 'general',
    name: '通用设置',
    icon: IconSettings
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

// 打开覆盖层窗口
const openOverlayWindow = (): void => {
  send('open-overlay-window')
}

// 切换开机自启
const toggleAutoStart = () => {
  settingsStore.autoStart = !settingsStore.autoStart
  // TODO: 发送到主进程设置开机自启
  console.log('开机自启:', settingsStore.autoStart)
}

// 更新最大历史记录数量
const updateMaxHistorySize = (value: number): void => {
  // 确保值在合理范围内
  const clampedValue = Math.max(10, Math.min(1000, value))
  settingsStore.maxHistorySize = clampedValue
  console.log('最大历史记录数量:', clampedValue)
}

// 更新最大保存时间
const updateMaxKeepTime = (value: number): void => {
  // 确保值在合理范围内，-1 表示不限制，否则最少1天，最多365天
  let clampedValue = value
  if (value !== -1) {
    clampedValue = Math.max(1, Math.min(365, value))
  }
  settingsStore.maxKeepTime = clampedValue
  console.log('最大保存时间:', clampedValue === -1 ? '不限制' : `${clampedValue}天`)
}

// 设置全局快捷键（存储 Electron Accelerator 格式）
const setGlobalShortcut = (electronAccelerator: string): void => {
  // 存储 Electron Accelerator 格式，用于实际的快捷键注册
  settingsStore.globalShortcut = electronAccelerator
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
    const oldShortcut = settingsStore.globalShortcut
    settingsStore.globalShortcut = ''
    console.log('快捷键已清除:', convertElectronAcceleratorToDisplay(oldShortcut), '→ 未设置')
  }
}

// 标签管理相关状态
const allTags = ref<TagGroup[]>([])
const isLoadingTags = ref(false)
const isCreatingTag = ref(false)
const isEditingTag = ref(false)
const editingTagId = ref<string | null>(null)

// 新建/编辑标签表单
const tagForm = ref({
  name: '',
  color: '#3b82f6',
  description: ''
})

// 预设颜色选项
const presetColors = [
  '#3b82f6', // 蓝色
  '#10b981', // 绿色
  '#8b5cf6', // 紫色
  '#f59e0b', // 橙色
  '#ef4444', // 红色
  '#06b6d4', // 青色
  '#84cc16', // 柠檬绿
  '#f97316', // 深橙色
  '#ec4899', // 粉色
  '#6366f1'  // 靛蓝色
]

// 加载所有标签
const loadAllTags = async () => {
  try {
    isLoadingTags.value = true
    allTags.value = await tags.getAllTags()
    console.log('标签加载成功:', allTags.value)
  } catch (error) {
    console.error('加载标签失败:', error)
    // 可以在这里添加错误提示
  } finally {
    isLoadingTags.value = false
  }
}

// 重置标签表单
const resetTagForm = () => {
  tagForm.value = {
    name: '',
    color: '#3b82f6',
    description: ''
  }
  isCreatingTag.value = false
  isEditingTag.value = false
  editingTagId.value = null
}

// 开始创建新标签
const startCreateTag = () => {
  resetTagForm()
  isCreatingTag.value = true
}

// 开始编辑标签
const startEditTag = (tag: TagGroup) => {
  tagForm.value = {
    name: tag.name,
    color: tag.color,
    description: tag.description || ''
  }
  editingTagId.value = tag.id
  isEditingTag.value = true
}

// 保存标签（创建或更新）
const saveTag = async () => {
  if (!tagForm.value.name.trim()) {
    alert('请输入标签名称')
    return
  }

  try {
    if (isCreatingTag.value) {
      // 创建新标签
      await tags.createTag(
        tagForm.value.name.trim(),
        tagForm.value.color,
        tagForm.value.description.trim() || undefined
      )
      console.log('标签创建成功')
    } else if (isEditingTag.value && editingTagId.value) {
      // 更新标签
      await tags.updateTag(editingTagId.value, {
        name: tagForm.value.name.trim(),
        color: tagForm.value.color,
        description: tagForm.value.description.trim() || undefined
      })
      console.log('标签更新成功')
    }

    // 重新加载标签列表
    await loadAllTags()
    resetTagForm()
  } catch (error) {
    console.error('保存标签失败:', error)
    alert('保存标签失败: ' + (error as Error).message)
  }
}

// 删除标签
const deleteTag = async (tag: TagGroup) => {
  if (!confirm(`确定要删除标签"${tag.name}"吗？这将同时移除所有相关的标签绑定。`)) {
    return
  }

  try {
    await tags.deleteTag(tag.id)
    console.log('标签删除成功')
    await loadAllTags()
  } catch (error) {
    console.error('删除标签失败:', error)
    alert('删除标签失败: ' + (error as Error).message)
  }
}

// 取消编辑
const cancelEdit = () => {
  resetTagForm()
}

// 格式化时间显示
const formatDate = (timestamp: number): string => {
  return new Date(timestamp).toLocaleString('zh-CN')
}

// 切换剪贴板监听
const toggleClipboardListening = async () => {
  try {
    const newState = !settingsStore.clipboardListening
    
    if (newState) {
      const success = await clipboard.startListening()
      if (success) {
        settingsStore.clipboardListening = true
        console.log('剪贴板监听已开启')
      } else {
        console.error('开启剪贴板监听失败')
      }
    } else {
      const success = await clipboard.stopListening()
      if (success) {
        settingsStore.clipboardListening = false
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
      settingsStore.clipboardListening = status.isListening
      console.log('剪贴板监听状态已同步:', status.isListening)
    }
  } catch (error) {
    console.error('获取剪贴板监听状态失败:', error)
  }
}

// 组件挂载时加载标签和同步剪贴板状态
onMounted(() => {
  loadAllTags()
  syncClipboardListeningStatus()
})

// 组件卸载时清理事件监听
onUnmounted(() => {
  if (isListeningForShortcut.value) {
    stopListeningForShortcut()
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
        <div v-for="section in settingSections" :key="section.id" class="nav-item"
          :class="{ active: activeSection === section.id }" @click="setActiveSection(section.id)">
          <component :is="section.icon" class="nav-icon" />
          <span class="nav-text">{{ section.name }}</span>
        </div>
      </nav>
    </div>

    <!-- 右侧内容区域 -->
    <div class="main-content">
      <!-- 通用设置 -->
      <div v-if="activeSection === 'general'" class="content-section">
        <div class="section-header">
          <h3 class="section-title">通用设置</h3>
          <p class="section-subtitle">配置应用的基本行为</p>
        </div>

        <div class="settings-grid">
          <!-- 开机自启 -->
          <div class="setting-card">
            <div class="setting-info">
              <h4 class="setting-title">开机自启</h4>
              <p class="setting-description">系统启动时自动运行 Super Paste</p>
            </div>
            <div class="setting-control">
              <label class="toggle-switch">
                <input type="checkbox" :checked="settingsStore.autoStart" @change="toggleAutoStart">
                <span class="toggle-slider"></span>
              </label>
            </div>
          </div>

          <!-- 最大历史记录数量 -->
          <div class="setting-card">
            <div class="setting-info">
              <h4 class="setting-title">最大历史记录数量</h4>
              <p class="setting-description">设置剪贴板历史记录的最大保存数量 (10-1000)</p>
            </div>
            <div class="setting-control">
              <div class="number-input-container">
                <input type="number" :value="settingsStore.maxHistorySize"
                  @input="updateMaxHistorySize(parseInt(($event.target as HTMLInputElement).value) || 100)" min="10"
                  max="1000" step="10" class="number-input">
                <span class="number-unit">条</span>
              </div>
            </div>
          </div>

          <!-- 最大保存时间 -->
          <div class="setting-card">
            <div class="setting-info">
              <h4 class="setting-title">历史记录保存时间</h4>
              <p class="setting-description">设置历史记录的最大保存时间，-1表示不限制 (1-365天)</p>
            </div>
            <div class="setting-control">
              <div class="number-input-container">
                <input type="number" :value="settingsStore.maxKeepTime"
                  @input="updateMaxKeepTime(parseInt(($event.target as HTMLInputElement).value) || -1)" min="-1"
                  max="365" step="1" class="number-input">
                <span class="number-unit">{{ settingsStore.maxKeepTime === -1 ? '不限制' : '天' }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 标签管理 -->
      <div v-if="activeSection === 'tags'" class="content-section">
        <div class="section-header">
          <h3 class="section-title">标签管理</h3>
          <p class="section-subtitle">创建和管理剪贴板内容的标签分类</p>
        </div>

        <!-- 创建新标签按钮 -->
        <div class="tag-actions">
          <button class="create-tag-button" @click="startCreateTag" :disabled="isCreatingTag || isEditingTag">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19"></line>
              <line x1="5" y1="12" x2="19" y2="12"></line>
            </svg>
            新建标签
          </button>
        </div>

        <!-- 标签表单 -->
        <div v-if="isCreatingTag || isEditingTag" class="tag-form-card">
          <div class="tag-form-header">
            <h4 class="tag-form-title">{{ isCreatingTag ? '创建新标签' : '编辑标签' }}</h4>
          </div>

          <div class="tag-form-content">
            <!-- 标签名称 -->
            <div class="form-group">
              <label class="form-label">标签名称</label>
              <input type="text" v-model="tagForm.name" class="form-input" placeholder="请输入标签名称" maxlength="20" />
            </div>

            <!-- 标签颜色 -->
            <div class="form-group">
              <label class="form-label">标签颜色</label>
              <div class="color-picker">
                <div class="color-options">
                  <div v-for="color in presetColors" :key="color" class="color-option"
                    :class="{ active: tagForm.color === color }" :style="{ backgroundColor: color }"
                    @click="tagForm.color = color"></div>
                </div>
                <input type="color" v-model="tagForm.color" class="color-input" />
              </div>
            </div>

            <!-- 标签描述 -->
            <div class="form-group">
              <label class="form-label">描述（可选）</label>
              <textarea v-model="tagForm.description" class="form-textarea" placeholder="请输入标签描述" maxlength="100"
                rows="3"></textarea>
            </div>

            <!-- 表单按钮 -->
            <div class="form-actions">
              <button class="form-button primary" @click="saveTag">
                {{ isCreatingTag ? '创建' : '保存' }}
              </button>
              <button class="form-button secondary" @click="cancelEdit">
                取消
              </button>
            </div>
          </div>
        </div>

        <!-- 标签列表 -->
        <div class="tags-list">
          <div v-if="isLoadingTags" class="loading-state">
            <div class="loading-spinner"></div>
            <span>加载标签中...</span>
          </div>

          <div v-else-if="allTags.length === 0" class="empty-state">
            <div class="empty-icon">🏷️</div>
            <p class="empty-text">暂无标签</p>
            <p class="empty-hint">点击"新建标签"创建第一个标签</p>
          </div>

          <div v-else class="tags-grid">
            <div v-for="tag in allTags" :key="tag.id" class="tag-card">
              <div class="tag-card-header">
                <div class="tag-info">
                  <div class="tag-color-dot" :style="{ backgroundColor: tag.color }"></div>
                  <h4 class="tag-name">{{ tag.name }}</h4>
                </div>
                <div class="tag-actions-menu">
                  <button class="tag-action-button" @click="startEditTag(tag)">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
                      <path d="m18.5 2.5 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
                    </svg>
                  </button>
                  <button class="tag-action-button danger" @click="deleteTag(tag)">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                      <polyline points="3,6 5,6 21,6"></polyline>
                      <path d="m19,6v14a2,2 0 0,1 -2,2H7a2,2 0 0,1 -2,-2V6m3,0V4a2,2 0 0,1 2,-2h4a2,2 0 0,1 2,2v2">
                      </path>
                    </svg>
                  </button>
                </div>
              </div>

              <div v-if="tag.description" class="tag-description">
                {{ tag.description }}
              </div>

              <div class="tag-meta">
                <span class="tag-date">创建于 {{ formatDate(tag.createdAt) }}</span>
                <span v-if="tag.updatedAt !== tag.createdAt" class="tag-date">
                  更新于 {{ formatDate(tag.updatedAt) }}
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 快捷键设置 -->
      <div v-if="activeSection === 'shortcuts'" class="content-section">
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

      <!-- 调试设置 -->
      <div v-if="activeSection === 'debug'" class="content-section">
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
                <input type="checkbox" :checked="settingsStore.clipboardListening" @change="toggleClipboardListening">
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
              <button class="test-button" @click="openOverlayWindow">
                打开测试
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- 关于 -->
      <div v-if="activeSection === 'about'" class="content-section">
        <div class="section-header">
          <h3 class="section-title">关于 Super Paste</h3>
          <p class="section-subtitle">应用信息和版本详情</p>
        </div>

        <div class="about-content">
          <div class="app-info">
            <h4 class="app-name">Super Paste</h4>
            <p class="app-version">版本 1.0.0</p>
            <p class="app-description">基于 Electron + Vue 的智能粘贴工具</p>
          </div>

          <Versions />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-container {
  display: flex;
  height: 100vh;
  background: #dfd8d6;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  padding: 16px;
  padding-left: 0;
  gap: 16px;
}

/* L形侧边栏 - 从顶部到底部 */
.sidebar {
  width: 240px;
  background: #dfd8d6;
  display: flex;
  flex-direction: column;
  padding: 24px 20px;
}

.sidebar-header {
  padding-bottom: 24px;
  border-bottom: 1px solid rgba(0, 0, 0, 0.08);
  margin-bottom: 24px;
}

.app-title {
  font-size: 18px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0 0 4px 0;
}

.app-subtitle {
  font-size: 14px;
  color: #666666;
  margin: 0;
}

.sidebar-nav {
  flex: 1;
}

.nav-item {
  display: flex;
  align-items: center;
  padding: 12px 16px;
  cursor: pointer;
  transition: all 0.2s ease;
  border-radius: 8px;
  margin: 2px 0;
}

.nav-item:hover {
  background: rgba(0, 0, 0, 0.05);
}

.nav-item.active {
  background: rgba(0, 0, 0, 0.08);
  color: #2d2d2d;
}

.nav-item.active .nav-text {
  color: #2d2d2d;
  font-weight: 500;
}

.nav-icon {
  width: 20px;
  height: 20px;
  margin-right: 12px;
  color: #666666;
  flex-shrink: 0;
}

.nav-item.active .nav-icon {
  color: #2d2d2d;
}

.nav-text {
  font-size: 14px;
  color: #666666;
}

/* 主内容区域 - 圆角浮动卡片 */
.main-content {
  flex: 1;
  background: #ffffff;
  border-radius: 16px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.08);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.content-section {
  padding: 32px;
  flex: 1;
  overflow-y: auto;
}

.section-header {
  margin-bottom: 32px;
}

.section-title {
  font-size: 24px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0 0 8px 0;
}

.section-subtitle {
  font-size: 14px;
  color: #666666;
  margin: 0;
}

/* 设置网格 */
.settings-grid {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.setting-card {
  background: #fafafa;
  border: 1px solid #f0f0f0;
  border-radius: 12px;
  padding: 24px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  transition: all 0.2s ease;
}

.setting-card:hover {
  background: #f5f5f5;
  border-color: #e8e8e8;
  /* transform: translateY(-1px); */
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.05);
}

.setting-info {
  flex: 1;
}

.setting-title {
  font-size: 16px;
  font-weight: 500;
  color: #2d2d2d;
  margin: 0 0 4px 0;
}

.setting-description {
  font-size: 14px;
  color: #666666;
  margin: 0;
}

.setting-control {
  margin-left: 20px;
}

/* 开关样式 */
.toggle-switch {
  position: relative;
  display: inline-block;
  width: 48px;
  height: 24px;
}

.toggle-switch input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background-color: #cccccc;
  transition: 0.3s;
  border-radius: 24px;
}

.toggle-slider:before {
  position: absolute;
  content: "";
  height: 18px;
  width: 18px;
  left: 3px;
  bottom: 3px;
  background-color: white;
  transition: 0.3s;
  border-radius: 50%;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
}

input:checked+.toggle-slider {
  background-color: #4a9eff;
}

input:checked+.toggle-slider:before {
  transform: translateX(24px);
}

/* 测试按钮 */
.test-button {
  background: #4a9eff;
  color: white;
  border: none;
  border-radius: 8px;
  padding: 8px 16px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.test-button:hover {
  background: #3d8beb;
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(74, 158, 255, 0.3);
}

/* 数字输入框 */
.number-input-container {
  display: flex;
  align-items: center;
  gap: 8px;
}

.number-input {
  background: #ffffff;
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 14px;
  color: #2d2d2d;
  width: 80px;
  text-align: center;
  transition: all 0.2s ease;
}

.number-input:focus {
  outline: none;
  border-color: #4a9eff;
  box-shadow: 0 0 0 3px rgba(74, 158, 255, 0.1);
}

.number-input:hover {
  border-color: #cccccc;
}

.number-unit {
  font-size: 14px;
  color: #666666;
  font-weight: 500;
}

/* 快捷键输入框 */
.shortcut-input {
  display: flex;
  flex-direction: column;
  gap: 12px;
  align-items: flex-start;
}

.shortcut-display {
  width: 100%;
}

.shortcut-field {
  background: #ffffff;
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 14px;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  color: #2d2d2d;
  min-width: 200px;
  width: 100%;
  transition: all 0.2s ease;
  cursor: default;
}

.shortcut-field:focus {
  outline: none;
  border-color: #4a9eff;
  box-shadow: 0 0 0 3px rgba(74, 158, 255, 0.1);
}

.shortcut-field.listening {
  border-color: #ff9500;
  background: #fff8f0;
  box-shadow: 0 0 0 3px rgba(255, 149, 0, 0.1);
  animation: pulse 2s infinite;
}

.shortcut-field.preview {
  color: #ff9500;
  font-weight: 500;
}

.shortcut-field.empty {
  color: #999999;
  font-style: italic;
}

@keyframes pulse {

  0%,
  100% {
    box-shadow: 0 0 0 3px rgba(255, 149, 0, 0.1);
  }

  50% {
    box-shadow: 0 0 0 3px rgba(255, 149, 0, 0.2);
  }
}

.shortcut-buttons {
  display: flex;
  align-items: center;
  gap: 12px;
}

.shortcut-button {
  border: none;
  border-radius: 6px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.shortcut-button.primary {
  background: #4a9eff;
  color: white;
}

.shortcut-button.primary:hover {
  background: #3d8beb;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(74, 158, 255, 0.3);
}

.shortcut-button.secondary {
  background: #f5f5f5;
  color: #666666;
  border: 1px solid #e0e0e0;
}

.shortcut-button.secondary:hover {
  background: #eeeeee;
  border-color: #d0d0d0;
}

.shortcut-button.danger {
  background: #ff4757;
  color: white;
  border: 1px solid #ff4757;
}

.shortcut-button.danger:hover {
  background: #ff3742;
  border-color: #ff3742;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(255, 71, 87, 0.3);
}

.shortcut-hint {
  font-size: 12px;
  color: #ff9500;
  font-style: italic;
}

.shortcut-error {
  background: #ffebee;
  border: 1px solid #ffcdd2;
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 12px;
  color: #c62828;
  margin-top: 8px;
}

.shortcut-requirements {
  margin-top: 16px;
  padding: 12px;
  background: #f8f9fa;
  border: 1px solid #e9ecef;
  border-radius: 6px;
}

.requirement-text {
  font-size: 12px;
  font-weight: 500;
  color: #495057;
  margin: 0 0 8px 0;
}

.requirement-list {
  margin: 0;
  padding-left: 16px;
  font-size: 11px;
  color: #6c757d;
}

.requirement-list li {
  margin-bottom: 4px;
}

.requirement-list li:last-child {
  margin-bottom: 0;
}

/* 关于页面 */
.about-content {
  display: flex;
  flex-direction: column;
  gap: 32px;
}

.app-info {
  background: #fafafa;
  border: 1px solid #f0f0f0;
  border-radius: 12px;
  padding: 32px;
  text-align: center;
}

.app-logo {
  width: 64px;
  height: 64px;
  margin-bottom: 16px;
}

.app-name {
  font-size: 20px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0 0 8px 0;
}

.app-version {
  font-size: 14px;
  color: #666666;
  margin: 0 0 12px 0;
}

.app-description {
  font-size: 14px;
  color: #666666;
  margin: 0;
}

/* 版本信息样式覆盖 */
:deep(.versions) {
  background: #fafafa;
  border: 1px solid #f0f0f0;
  border-radius: 12px;
  padding: 24px;
  list-style: none;
  margin: 0;
}

:deep(.versions li) {
  padding: 8px 0;
  border-bottom: 1px solid #f0f0f0;
  font-size: 14px;
  color: #666666;
}

:deep(.versions li:last-child) {
  border-bottom: none;
}

/* 标签管理样式 */
.tag-actions {
  margin-bottom: 24px;
}

.create-tag-button {
  background: #4a9eff;
  color: white;
  border: none;
  border-radius: 8px;
  padding: 10px 16px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  display: flex;
  align-items: center;
  gap: 8px;
}

.create-tag-button:hover:not(:disabled) {
  background: #3d8beb;
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(74, 158, 255, 0.3);
}

.create-tag-button:disabled {
  background: #cccccc;
  cursor: not-allowed;
  transform: none;
  box-shadow: none;
}

/* 标签表单 */
.tag-form-card {
  background: #fafafa;
  border: 1px solid #f0f0f0;
  border-radius: 12px;
  padding: 24px;
  margin-bottom: 24px;
}

.tag-form-header {
  margin-bottom: 20px;
}

.tag-form-title {
  font-size: 18px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0;
}

.tag-form-content {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.form-label {
  font-size: 14px;
  font-weight: 500;
  color: #2d2d2d;
}

.form-input,
.form-textarea {
  background: #ffffff;
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  padding: 10px 12px;
  font-size: 14px;
  color: #2d2d2d;
  transition: all 0.2s ease;
  font-family: inherit;
}

.form-input:focus,
.form-textarea:focus {
  outline: none;
  border-color: #4a9eff;
  box-shadow: 0 0 0 3px rgba(74, 158, 255, 0.1);
}

.form-textarea {
  resize: vertical;
  min-height: 80px;
}

/* 颜色选择器 */
.color-picker {
  display: flex;
  align-items: center;
  gap: 16px;
}

.color-options {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.color-option {
  width: 32px;
  height: 32px;
  border-radius: 6px;
  cursor: pointer;
  border: 2px solid transparent;
  transition: all 0.2s ease;
}

.color-option:hover {
  transform: scale(1.1);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
}

.color-option.active {
  border-color: #2d2d2d;
  transform: scale(1.1);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
}

.color-input {
  width: 40px;
  height: 32px;
  border: 1px solid #e0e0e0;
  border-radius: 6px;
  cursor: pointer;
  background: none;
}

/* 表单按钮 */
.form-actions {
  display: flex;
  gap: 12px;
  margin-top: 8px;
}

.form-button {
  border: none;
  border-radius: 6px;
  padding: 10px 20px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
}

.form-button.primary {
  background: #4a9eff;
  color: white;
}

.form-button.primary:hover {
  background: #3d8beb;
  transform: translateY(-1px);
  box-shadow: 0 2px 8px rgba(74, 158, 255, 0.3);
}

.form-button.secondary {
  background: #f5f5f5;
  color: #666666;
  border: 1px solid #e0e0e0;
}

.form-button.secondary:hover {
  background: #eeeeee;
  border-color: #d0d0d0;
}

/* 标签列表 */
.tags-list {
  margin-top: 24px;
}

.loading-state,
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  text-align: center;
}

.loading-spinner {
  width: 24px;
  height: 24px;
  border: 2px solid #f0f0f0;
  border-left-color: #4a9eff;
  border-radius: 50%;
  animation: spin 1s linear infinite;
  margin-bottom: 12px;
}

.empty-icon {
  font-size: 48px;
  margin-bottom: 16px;
  opacity: 0.6;
}

.empty-text {
  font-size: 18px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0 0 8px 0;
}

.empty-hint {
  font-size: 14px;
  color: #666666;
  margin: 0;
}

/* 标签网格 */
.tags-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 16px;
}

.tag-card {
  background: #ffffff;
  border: 1px solid #f0f0f0;
  border-radius: 12px;
  padding: 20px;
  transition: all 0.2s ease;
}

.tag-card:hover {
  border-color: #e0e0e0;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
  transform: translateY(-1px);
}

.tag-card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 12px;
}

.tag-info {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
}

.tag-color-dot {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tag-name {
  font-size: 16px;
  font-weight: 600;
  color: #2d2d2d;
  margin: 0;
  word-break: break-word;
}

.tag-actions-menu {
  display: flex;
  gap: 4px;
  opacity: 0;
  transition: opacity 0.2s ease;
}

.tag-card:hover .tag-actions-menu {
  opacity: 1;
}

.tag-action-button {
  background: none;
  border: none;
  padding: 6px;
  border-radius: 4px;
  cursor: pointer;
  color: #666666;
  transition: all 0.2s ease;
  display: flex;
  align-items: center;
  justify-content: center;
}

.tag-action-button:hover {
  background: #f5f5f5;
  color: #2d2d2d;
}

.tag-action-button.danger:hover {
  background: #ffebee;
  color: #c62828;
}

.tag-description {
  font-size: 14px;
  color: #666666;
  line-height: 1.4;
  margin-bottom: 12px;
  word-break: break-word;
}

.tag-meta {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.tag-date {
  font-size: 12px;
  color: #999999;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* 响应式设计 */
@media (max-width: 768px) {
  .settings-container {
    flex-direction: column;
    padding: 8px;
    gap: 8px;
  }

  .sidebar {
    width: 100%;
    padding: 16px;
  }

  .content-section {
    padding: 20px;
  }

  .setting-card {
    flex-direction: column;
    align-items: flex-start;
    gap: 16px;
  }

  .setting-control {
    margin-left: 0;
    width: 100%;
  }

  .tags-grid {
    grid-template-columns: 1fr;
  }

  .tag-actions-menu {
    opacity: 1;
  }

  .form-actions {
    flex-direction: column;
  }

  .color-options {
    gap: 6px;
  }

  .color-option {
    width: 28px;
    height: 28px;
  }
}
</style>
