<script setup lang="ts">
import { clipboard } from '@renderer/api/renderer-api'

// Props
interface Props {
  settingsStore: any
}

const props = defineProps<Props>()

// 保存时间预设选项
const keepTimePresets = [
  { label: '不限制', value: -1 },
  { label: '周', value: 7 },
  { label: '月', value: 30 },
  { label: '季度', value: 90 },
  { label: '半年', value: 180 },
  { label: '一年', value: 365 }
]

// 滑块标记点
const sliderMarks = [
  { label: '7天', value: 7 },
  { label: '30天', value: 30 },
  { label: '90天', value: 90 },
  { label: '180天', value: 180 },
  { label: '365天', value: 365 }
]

type OverlayPosition = 'top' | 'bottom' | 'left' | 'right'

const overlayPositionOptions: Array<{ label: string; value: OverlayPosition }> = [
  { label: '上', value: 'top' },
  { label: '下', value: 'bottom' },
  { label: '左', value: 'left' },
  { label: '右', value: 'right' }
]

// 计算滑块标记位置的函数
const getMarkPosition = (value: number): number => {
  // 滑块范围是 1-365
  const min = 1
  const max = 365

  // 计算基础百分比
  const percentage = (value - min) / (max - min)

  // 尝试使用更精确的调整方法
  // 基于实际观察，HTML range元素的thumb定位遵循特定的模式

  // 对于不同的值，使用不同的调整策略
  // 继续向左微调
  if (percentage === 0) {
    return 1.6 // 最小值时的位置
  } else if (percentage === 1) {
    return 98.6 // 最大值时的位置
  } else {
    // 中间值使用线性插值
    return 1.6 + percentage * (98.6 - 1.6)
  }
}

// 切换开机自启
const toggleAutoStart = () => {
  props.settingsStore.autoStart = !props.settingsStore.autoStart
  // TODO: 发送到主进程设置开机自启
  console.log('开机自启:', props.settingsStore.autoStart)
}

// 更新最大历史记录数量
const updateMaxHistorySize = (value: number): void => {
  // 确保值在合理范围内
  const clampedValue = Math.max(10, Math.min(1000, value))
  props.settingsStore.maxHistorySize = clampedValue
  console.log('最大历史记录数量:', clampedValue)
}

// 更新最大保存时间
const updateMaxKeepTime = (value: number): void => {
  // 确保值在合理范围内，-1 表示不限制，否则最少1天，最多365天
  let clampedValue = value
  if (value !== -1) {
    clampedValue = Math.max(1, Math.min(365, value))
  }
  props.settingsStore.maxKeepTime = clampedValue
  console.log('最大保存时间:', clampedValue === -1 ? '不限制' : `${clampedValue}天`)
}

const normalizeOverlayPosition = (value: unknown): OverlayPosition => {
  if (value === 'top' || value === 'bottom' || value === 'left' || value === 'right') {
    return value
  }
  return 'bottom'
}

const getCurrentOverlayPosition = (): OverlayPosition => {
  return normalizeOverlayPosition(props.settingsStore.overlayPosition)
}

const updateOverlayPosition = (value: OverlayPosition): void => {
  const normalized = normalizeOverlayPosition(value)
  props.settingsStore.overlayPosition = normalized
  console.log('overlay 出现位置:', normalized)
}

props.settingsStore.overlayPosition = getCurrentOverlayPosition()

// 清除历史记录
const clearHistory = async () => {
  if (!confirm('确定要清除所有剪贴板历史记录吗？此操作不可撤销。')) {
    return
  }

  try {
    const success = await clipboard.clearHistory()
    if (success) {
      alert('历史记录已清除')
      console.log('剪贴板历史记录已清除')
    } else {
      alert('清除历史记录失败')
      console.error('清除历史记录失败')
    }
  } catch (error) {
    console.error('清除历史记录失败:', error)
    alert('清除历史记录失败: ' + (error as Error).message)
  }
}
</script>

<template>
  <div class="content-section">
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
            <input type="checkbox" :checked="settingsStore.autoStart" @change="toggleAutoStart" />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- Overlay 出现位置 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">Overlay 出现位置</h4>
          <p class="setting-description">设置快捷面板从屏幕的哪个方向出现，默认底部</p>
        </div>
        <div class="setting-control">
          <div class="overlay-position-grid">
            <button
              v-for="option in overlayPositionOptions"
              :key="option.value"
              type="button"
              :class="[
                'overlay-position-button',
                { active: getCurrentOverlayPosition() === option.value }
              ]"
              @click="updateOverlayPosition(option.value)"
            >
              <span class="overlay-position-label">{{ option.label }}</span>
            </button>
          </div>
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
            <input
              type="number"
              :value="settingsStore.maxHistorySize"
              @input="
                updateMaxHistorySize(parseInt(($event.target as HTMLInputElement).value) || 100)
              "
              min="10"
              max="1000"
              step="10"
              class="number-input"
            />
            <span class="number-unit">条</span>
          </div>
        </div>
      </div>

      <!-- 最大保存时间 -->
      <div class="setting-card keep-time-card">
        <div class="setting-info">
          <h4 class="setting-title">历史记录保存时间</h4>
          <p class="setting-description">设置历史记录的最大保存时间</p>
        </div>
        <div class="setting-control keep-time-control">
          <!-- 快速选择按钮和输入框在同一行 -->
          <div class="keep-time-inline-section">
            <!-- 快速选择按钮 -->
            <div class="keep-time-presets-compact">
              <button
                v-for="preset in keepTimePresets"
                :key="preset.value"
                :class="[
                  'preset-button-compact',
                  { active: settingsStore.maxKeepTime === preset.value }
                ]"
                @click="updateMaxKeepTime(preset.value)"
              >
                {{ preset.label }}
              </button>
            </div>

            <!-- 输入框在右边 -->
            <div class="number-input-container">
              <input
                type="number"
                :value="settingsStore.maxKeepTime === -1 ? '' : settingsStore.maxKeepTime"
                @input="
                  updateMaxKeepTime(parseInt(($event.target as HTMLInputElement).value) || -1)
                "
                :placeholder="settingsStore.maxKeepTime === -1 ? '不限制' : '输入天数'"
                min="1"
                max="365"
                step="1"
                class="number-input"
                :disabled="settingsStore.maxKeepTime === -1"
              />
              <span class="number-unit">{{
                settingsStore.maxKeepTime === -1 ? '不限制' : '天'
              }}</span>
            </div>
          </div>

          <!-- 紧凑的滑块控制 -->
          <div class="keep-time-slider-compact" v-if="settingsStore.maxKeepTime !== -1">
            <input
              type="range"
              :value="settingsStore.maxKeepTime"
              @input="updateMaxKeepTime(parseInt(($event.target as HTMLInputElement).value))"
              min="1"
              max="365"
              step="1"
              class="keep-time-slider"
            />
            <div class="slider-marks">
              <div
                class="slider-mark"
                v-for="mark in sliderMarks"
                :key="mark.value"
                :style="{ left: `${getMarkPosition(mark.value)}%` }"
              >
                <span class="mark-label">{{ mark.label }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 清除历史记录 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">清除历史记录</h4>
          <p class="setting-description">清除所有剪贴板历史记录，此操作不可撤销</p>
        </div>
        <div class="setting-control">
          <button class="clear-history-button" @click="clearHistory">清除历史</button>
        </div>
      </div>
    </div>
  </div>
</template>
