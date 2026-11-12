<script setup lang="ts">
import { useSettingsStore } from '@renderer/stores/common'
import { clipboard } from '@renderer/api/renderer-api'
import { confirmAction, showMessage } from '@renderer/api/dialog'
import { COLOR_MODE_OPTIONS, COLOR_MODE_TEXT, isColorMode } from '@shared/color-mode'
import SelectControl from '@renderer/components/SelectControl.vue'
import NumberInput from '@renderer/components/NumberInput.vue'
import {
  KEEP_TIME_RANGE,
  OVERLAY_POSITION_OPTIONS,
  SETTINGS_LABELS
} from '@renderer/constants/desktop-ui'
import IconCheck from '~icons/mdi/check'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { executeTransition, TransitionPresets, usePreferredReducedMotion } from '@vueuse/core'

const settingsStore = useSettingsStore()
const displayedKeepTime = ref(settingsStore.maxKeepTime)
const reducedMotion = usePreferredReducedMotion()
let keepTimeTransitionId = 0
let animateKeepTimeChange = false

// 动画只更新显示值，配置直接写入目标天数；拖动和键盘调整保持即时响应。
watch(
  [() => settingsStore.maxKeepTime, reducedMotion],
  ([value, motion], [previousValue]) => {
    const id = ++keepTimeTransitionId
    if (!animateKeepTimeChange || value === -1 || previousValue === -1 || motion === 'reduce') {
      displayedKeepTime.value = value
      return
    }
    void executeTransition(displayedKeepTime, displayedKeepTime.value, value, {
      duration: 200,
      transition: TransitionPresets.easeOutCubic,
      abort: () => id !== keepTimeTransitionId
    })
  },
  { flush: 'sync' }
)

onScopeDispose(() => {
  keepTimeTransitionId++
})

const sliderKeepTime = computed(() => Math.round(displayedKeepTime.value))

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

type OverlayPosition = (typeof OVERLAY_POSITION_OPTIONS)[number]['value']

const getKeepTimePercentage = (value: number): number =>
  ((value - KEEP_TIME_RANGE.min) / (KEEP_TIME_RANGE.max - KEEP_TIME_RANGE.min)) * 100

// 更新最大历史记录数量
const updateMaxHistorySize = (value: number | undefined): void => {
  if (value === undefined) return
  // 确保值在合理范围内
  const clampedValue = Math.max(10, Math.min(1000, value))
  settingsStore.$patch({ maxHistorySize: clampedValue })
}

const updateKeepTimeInput = (value: number | undefined): void => {
  updateMaxKeepTime(value ?? -1)
}

// 更新最大保存时间
const updateMaxKeepTime = (value: number, animate = false): void => {
  // 确保值在合理范围内，-1 表示不限制，否则最少1天，最多365天
  let clampedValue = value
  if (value !== -1) {
    clampedValue = Math.max(KEEP_TIME_RANGE.min, Math.min(KEEP_TIME_RANGE.max, value))
  }
  if (!animate) {
    keepTimeTransitionId++
    displayedKeepTime.value = clampedValue
  }
  animateKeepTimeChange = animate
  settingsStore.$patch({ maxKeepTime: clampedValue })
  animateKeepTimeChange = false
}

const normalizeOverlayPosition = (value: unknown): OverlayPosition => {
  if (value === 'top' || value === 'bottom' || value === 'left' || value === 'right') {
    return value
  }
  return 'bottom'
}

const getCurrentOverlayPosition = (): OverlayPosition => {
  return normalizeOverlayPosition(settingsStore.overlayPosition)
}

const overlayPositionIndex = computed(() =>
  OVERLAY_POSITION_OPTIONS.findIndex((option) => option.value === getCurrentOverlayPosition())
)

const updateColorMode = (value: string): void => {
  if (isColorMode(value)) settingsStore.$patch({ colorMode: value })
}

const updateAutoStart = (value: boolean): void => {
  settingsStore.$patch({ autoStart: value })
}

const updateSilentStart = (value: boolean): void => {
  settingsStore.$patch({ silentStart: value })
}

const updateOverlayPosition = (value: OverlayPosition): void => {
  const normalized = normalizeOverlayPosition(value)
  settingsStore.$patch({ overlayPosition: normalized })
}

settingsStore.$patch({ overlayPosition: getCurrentOverlayPosition() })

// 清除历史记录
const clearHistory = async (): Promise<void> => {
  if (!(await confirmAction('确定要清除所有剪贴板历史记录吗？此操作不可撤销。'))) {
    return
  }

  try {
    const success = await clipboard.clearHistory()
    if (success) {
      await showMessage('历史记录已清除')
    } else {
      await showMessage('清除历史记录失败')
      console.error('清除历史记录失败')
    }
  } catch (error) {
    console.error('清除历史记录失败:', error)
    await showMessage('清除历史记录失败: ' + (error as Error).message)
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
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">{{ COLOR_MODE_TEXT.title }}</h4>
          <p class="setting-description">{{ COLOR_MODE_TEXT.description }}</p>
        </div>
        <div class="setting-control">
          <SelectControl
            :model-value="settingsStore.colorMode"
            :options="COLOR_MODE_OPTIONS"
            class="select-input"
            :aria-label="COLOR_MODE_TEXT.title"
            @update:model-value="updateColorMode"
          />
        </div>
      </div>
      <!-- 开机自启 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">开机自启</h4>
          <p class="setting-description">系统启动时自动运行 Super Paste</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input
              :checked="settingsStore.autoStart"
              type="checkbox"
              @change="updateAutoStart(($event.target as HTMLInputElement).checked)"
            />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">静默启动</h4>
          <p class="setting-description">启动时不自动打开设置窗口，下次启动生效</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input
              :checked="settingsStore.silentStart"
              type="checkbox"
              aria-label="静默启动"
              @change="updateSilentStart(($event.target as HTMLInputElement).checked)"
            />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- Overlay 出现位置 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">{{ SETTINGS_LABELS.overlayPosition }}</h4>
          <p class="setting-description">设置快捷面板从屏幕的哪个方向出现，默认底部</p>
        </div>
        <div class="setting-control">
          <div
            class="overlay-position-grid"
            role="group"
            :aria-label="SETTINGS_LABELS.overlayPosition"
            :style="{
              '--position-index': overlayPositionIndex,
              '--position-count': OVERLAY_POSITION_OPTIONS.length
            }"
          >
            <span class="overlay-position-indicator" aria-hidden="true" />
            <button
              v-for="option in OVERLAY_POSITION_OPTIONS"
              :key="option.value"
              type="button"
              :class="[
                'overlay-position-button',
                `overlay-position-button--${option.value}`,
                { active: getCurrentOverlayPosition() === option.value }
              ]"
              :aria-pressed="getCurrentOverlayPosition() === option.value"
              @click="updateOverlayPosition(option.value)"
            >
              <span class="overlay-position-preview" aria-hidden="true">
                <span class="overlay-position-edge" />
              </span>
              <span class="overlay-position-label">{{ option.label }}</span>
              <Transition name="position-check">
                <IconCheck
                  v-if="getCurrentOverlayPosition() === option.value"
                  class="overlay-position-check"
                  aria-hidden="true"
                />
              </Transition>
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
            <NumberInput
              :model-value="settingsStore.maxHistorySize"
              :min="10"
              :max="1000"
              :step="10"
              :empty-value="100"
              class="number-input"
              aria-label="最大历史记录数量"
              @update:model-value="updateMaxHistorySize"
            />
            <span class="number-unit">条</span>
          </div>
        </div>
      </div>

      <!-- 最大保存时间 -->
      <div class="setting-card keep-time-card">
        <div class="setting-info">
          <h4 class="setting-title">{{ SETTINGS_LABELS.keepTime }}</h4>
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
                :aria-pressed="settingsStore.maxKeepTime === preset.value"
                @click="updateMaxKeepTime(preset.value, true)"
              >
                {{ preset.label }}
              </button>
            </div>

            <!-- 输入框在右边 -->
            <div class="number-input-container">
              <NumberInput
                :model-value="
                  settingsStore.maxKeepTime === -1 ? undefined : settingsStore.maxKeepTime
                "
                :placeholder="settingsStore.maxKeepTime === -1 ? '不限制' : '输入天数'"
                :min="KEEP_TIME_RANGE.min"
                :max="KEEP_TIME_RANGE.max"
                :step="1"
                :empty-value="-1"
                class="number-input"
                :aria-label="SETTINGS_LABELS.keepTime"
                :disabled="settingsStore.maxKeepTime === -1"
                @update:model-value="updateKeepTimeInput"
              />
              <span class="number-unit">{{
                settingsStore.maxKeepTime === -1 ? '不限制' : '天'
              }}</span>
            </div>
          </div>

          <!-- 紧凑的滑块控制 -->
          <div
            v-if="settingsStore.maxKeepTime !== -1"
            class="keep-time-slider-compact"
            :style="{ '--slider-progress': `${getKeepTimePercentage(sliderKeepTime)}%` }"
          >
            <div class="keep-time-slider-track" aria-hidden="true">
              <div class="keep-time-slider-fill" />
            </div>
            <input
              type="range"
              :value="sliderKeepTime"
              :min="KEEP_TIME_RANGE.min"
              :max="KEEP_TIME_RANGE.max"
              step="1"
              class="keep-time-slider"
              :aria-label="SETTINGS_LABELS.keepTime"
              :aria-valuenow="settingsStore.maxKeepTime"
              @input="updateMaxKeepTime(parseInt(($event.target as HTMLInputElement).value))"
            />
            <div class="slider-marks">
              <button
                v-for="mark in sliderMarks"
                :key="mark.value"
                type="button"
                class="slider-mark"
                :class="{ active: settingsStore.maxKeepTime === mark.value }"
                :aria-pressed="settingsStore.maxKeepTime === mark.value"
                :style="{ left: `${getKeepTimePercentage(mark.value)}%` }"
                @click="updateMaxKeepTime(mark.value, true)"
              >
                <span class="mark-label">{{ mark.label }}</span>
              </button>
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
