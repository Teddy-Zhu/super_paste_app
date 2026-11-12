<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import IconChevronDown from '~icons/mdi/chevron-down'
import { FORM_CONTROL_TEXT } from '@renderer/constants/form-controls'
import { useFloatingPopover } from '@renderer/composables/useFloatingPopover'

const props = withDefaults(
  defineProps<{ modelValue: string; ariaLabel?: string; disabled?: boolean }>(),
  { disabled: false }
)
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const id = useId()
const trigger = ref<HTMLButtonElement | null>(null)
const panel = ref<HTMLElement | null>(null)
const teleportTarget = ref<HTMLElement | string>('body')
const open = ref(false)
const hue = ref(0)
const saturation = ref(72)
const lightness = ref(52)
const hexDraft = ref('')
const hexInput = ref<HTMLInputElement | null>(null)
const invalidHex = ref(false)
const selectedHex = ref(props.modelValue.toLowerCase())
const closeForScroll = (): void => {
  closePicker()
}
const { style: panelStyle } = useFloatingPopover(open, trigger, panel, 268, closeForScroll)

type Hsl = { h: number; s: number; l: number }

const parseHex = (input: string): [number, number, number] | undefined => {
  const match = /^#?([\da-f]{3}|[\da-f]{6})$/i.exec(input.trim())
  if (!match) return undefined
  const value = match[1].length === 3 ? [...match[1]].map((part) => part + part).join('') : match[1]
  return [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16)) as [
    number,
    number,
    number
  ]
}

const rgbToHsl = ([red, green, blue]: [number, number, number]): Hsl => {
  const r = red / 255
  const g = green / 255
  const b = blue / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const delta = max - min
  let h = 0
  let s = 0
  const l = (max + min) / 2
  if (delta !== 0) {
    s = delta / (1 - Math.abs(2 * l - 1))
    if (max === r) h = ((g - b) / delta) % 6
    else if (max === g) h = (b - r) / delta + 2
    else h = (r - g) / delta + 4
    h *= 60
    if (h < 0) h += 360
  }
  return { h, s: s * 100, l: l * 100 }
}

const rgbToHex = ([red, green, blue]: [number, number, number]): string =>
  `#${[red, green, blue].map((value) => value.toString(16).padStart(2, '0')).join('')}`

const hslToHex = (h: number, s: number, l: number): string => {
  const saturationValue = s / 100
  const lightnessValue = l / 100
  const chroma = (1 - Math.abs(2 * lightnessValue - 1)) * saturationValue
  const segment = h / 60
  const secondary = chroma * (1 - Math.abs((segment % 2) - 1))
  const [red, green, blue] =
    segment < 1
      ? [chroma, secondary, 0]
      : segment < 2
        ? [secondary, chroma, 0]
        : segment < 3
          ? [0, chroma, secondary]
          : segment < 4
            ? [0, secondary, chroma]
            : segment < 5
              ? [secondary, 0, chroma]
              : [chroma, 0, secondary]
  const offset = lightnessValue - chroma / 2
  return `#${[red, green, blue]
    .map((value) =>
      Math.round((value + offset) * 255)
        .toString(16)
        .padStart(2, '0')
    )
    .join('')}`
}

const currentColor = computed(() => selectedHex.value)

const setFromHex = (value: string): void => {
  if (props.disabled) return
  const rgb = parseHex(value)
  invalidHex.value = !rgb
  hexInput.value?.setCustomValidity(invalidHex.value ? FORM_CONTROL_TEXT.invalidColor : '')
  if (!rgb) return
  const hsl = rgbToHsl(rgb)
  hue.value = hsl.h
  saturation.value = hsl.s
  lightness.value = hsl.l
  selectedHex.value = rgbToHex(rgb)
  hexDraft.value = selectedHex.value.toUpperCase()
  emit('update:modelValue', selectedHex.value)
}

const updateHsl = (): void => {
  if (props.disabled) return
  invalidHex.value = false
  hexInput.value?.setCustomValidity('')
  selectedHex.value = hslToHex(hue.value, saturation.value, lightness.value)
  hexDraft.value = selectedHex.value.toUpperCase()
  emit('update:modelValue', selectedHex.value)
}

const onHexInput = (): void => {
  invalidHex.value = false
  hexInput.value?.setCustomValidity('')
}

const openPicker = (): void => {
  if (props.disabled) return
  // A modal dialog makes body portals inert; keep its picker in the same top layer.
  teleportTarget.value = trigger.value?.closest('dialog') || document.body
  open.value = true
  void nextTick(() => {
    if (open.value)
      panel.value?.querySelector<HTMLInputElement>('input')?.focus({ preventScroll: true })
  })
}
const closePicker = (returnFocus = false): void => {
  if (open.value) {
    if (!returnFocus && parseHex(hexDraft.value)) setFromHex(hexDraft.value)
    else {
      hexDraft.value = selectedHex.value.toUpperCase()
      invalidHex.value = false
      hexInput.value?.setCustomValidity('')
    }
  }
  open.value = false
  if (returnFocus) trigger.value?.focus()
}
const onPointerDown = (event: PointerEvent): void => {
  const target = event.target
  if (
    target instanceof Node &&
    !trigger.value?.contains(target) &&
    !panel.value?.contains(target)
  ) {
    closePicker()
  }
}
const onFocusIn = (event: FocusEvent): void => {
  const target = event.target
  if (
    target instanceof Node &&
    !trigger.value?.contains(target) &&
    !panel.value?.contains(target)
  ) {
    closePicker()
  }
}
const onKeydown = (event: KeyboardEvent): void => {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    closePicker(true)
  }
}

watch(
  () => props.modelValue,
  (value) => {
    const rgb = parseHex(value)
    if (!rgb) return
    const hsl = rgbToHsl(rgb)
    hue.value = hsl.h
    saturation.value = hsl.s
    lightness.value = hsl.l
    selectedHex.value = rgbToHex(rgb)
    hexDraft.value = selectedHex.value.toUpperCase()
    invalidHex.value = false
    hexInput.value?.setCustomValidity('')
  },
  { immediate: true }
)
watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('pointerdown', onPointerDown, true)
    document.addEventListener('focusin', onFocusIn, true)
  } else {
    document.removeEventListener('pointerdown', onPointerDown, true)
    document.removeEventListener('focusin', onFocusIn, true)
  }
})
watch(
  () => props.disabled,
  (disabled) => {
    if (disabled) closePicker()
  }
)
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onPointerDown, true)
  document.removeEventListener('focusin', onFocusIn, true)
})
</script>

<template>
  <div class="color-picker-control">
    <button
      ref="trigger"
      type="button"
      class="color-picker-control__trigger"
      :aria-label="ariaLabel || `${FORM_CONTROL_TEXT.color} ${modelValue}`"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="open ? `${id}-panel` : undefined"
      :disabled="disabled"
      @click="open ? closePicker() : openPicker()"
      @keydown="onKeydown"
    >
      <span class="color-picker-control__swatch" :style="{ backgroundColor: modelValue }" />
      <IconChevronDown aria-hidden="true" />
    </button>
    <Teleport :to="teleportTarget">
      <section
        v-if="open"
        :id="`${id}-panel`"
        ref="panel"
        class="color-picker-control__panel"
        role="dialog"
        :aria-label="ariaLabel || FORM_CONTROL_TEXT.chooseColor"
        :style="panelStyle"
        @keydown="onKeydown"
      >
        <div class="color-picker-control__preview">
          <span :style="{ backgroundColor: currentColor }" />
          <strong>{{ currentColor }}</strong>
        </div>
        <label class="color-picker-control__field">
          <span>{{ FORM_CONTROL_TEXT.hue }}</span>
          <input
            v-model.number="hue"
            type="range"
            min="0"
            max="360"
            step="1"
            class="color-picker-control__hue"
            :aria-label="FORM_CONTROL_TEXT.hue"
            @input="updateHsl"
          />
        </label>
        <label class="color-picker-control__field">
          <span>{{ FORM_CONTROL_TEXT.saturation }}</span>
          <input
            v-model.number="saturation"
            type="range"
            min="0"
            max="100"
            step="1"
            :aria-label="FORM_CONTROL_TEXT.saturation"
            @input="updateHsl"
          />
        </label>
        <label class="color-picker-control__field">
          <span>{{ FORM_CONTROL_TEXT.lightness }}</span>
          <input
            v-model.number="lightness"
            type="range"
            min="0"
            max="100"
            step="1"
            :aria-label="FORM_CONTROL_TEXT.lightness"
            @input="updateHsl"
          />
        </label>
        <label class="color-picker-control__hex">
          <span>{{ FORM_CONTROL_TEXT.hex }}</span>
          <input
            ref="hexInput"
            v-model="hexDraft"
            type="text"
            maxlength="7"
            spellcheck="false"
            autocomplete="off"
            :aria-label="FORM_CONTROL_TEXT.hexColor"
            :aria-invalid="invalidHex || undefined"
            :aria-describedby="invalidHex ? `${id}-error` : undefined"
            @input="onHexInput"
            @keydown.enter.prevent="setFromHex(hexDraft)"
            @blur="setFromHex(hexDraft)"
          />
        </label>
        <p v-if="invalidHex" :id="`${id}-error`" class="color-picker-control__error" role="alert">
          {{ FORM_CONTROL_TEXT.invalidColor }}
        </p>
      </section>
    </Teleport>
  </div>
</template>

<style>
.color-picker-control {
  display: inline-flex;
}

.color-picker-control__hex input[aria-invalid='true'] {
  border-color: var(--color-danger);
}

.color-picker-control__error {
  margin: 6px 0 0;
  color: var(--color-danger);
  font-size: 11px;
}

.color-picker-control__trigger {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: 40px;
  height: 28px;
  padding: 3px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  cursor: pointer;
}

.color-picker-control__trigger:hover:not(:disabled),
.color-picker-control__trigger:focus-visible {
  border-color: var(--color-primary);
  background: var(--color-bg-tertiary);
}

.color-picker-control__trigger:focus-visible {
  outline: none;
}

.color-picker-control__trigger:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.color-picker-control__swatch {
  flex: 1;
  height: 100%;
  min-width: 0;
  border: 1px solid rgba(0, 0, 0, 0.14);
  border-radius: 2px;
}

.color-picker-control__trigger svg {
  width: 12px;
  height: 12px;
  color: var(--color-text-secondary);
}

.color-picker-control__panel {
  z-index: 10000;
  display: grid;
  gap: 12px;
  overflow: auto;
  padding: 12px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-elevated);
}

.color-picker-control__preview {
  display: flex;
  align-items: center;
  gap: 9px;
  font-size: 12px;
}

.color-picker-control__preview span {
  width: 28px;
  height: 28px;
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-xs);
}

.color-picker-control__field,
.color-picker-control__hex {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
  color: var(--color-text-secondary);
  font-size: 11px;
}

.color-picker-control__field input[type='range'] {
  width: 100%;
  margin: 0;
  accent-color: var(--color-primary);
  -webkit-appearance: none;
  appearance: none;
  height: 16px;
  background: transparent;
}

.color-picker-control__field input[type='range']::-webkit-slider-runnable-track {
  height: 5px;
  border-radius: var(--radius-full);
  background: var(--color-control-border);
}

.color-picker-control__field input[type='range']::-webkit-slider-thumb {
  width: 14px;
  height: 14px;
  margin-top: -4.5px;
  border: 2px solid var(--color-bg-primary);
  border-radius: 50%;
  background: var(--color-primary);
  box-shadow: 0 0 0 1px var(--color-primary);
  -webkit-appearance: none;
  appearance: none;
}

.color-picker-control__field input[type='range']::-moz-range-track {
  height: 5px;
  border-radius: var(--radius-full);
  background: var(--color-control-border);
}

.color-picker-control__field input[type='range']::-moz-range-thumb {
  width: 10px;
  height: 10px;
  border: 2px solid var(--color-bg-primary);
  border-radius: 50%;
  background: var(--color-primary);
  box-shadow: 0 0 0 1px var(--color-primary);
}

.color-picker-control__field input.color-picker-control__hue::-webkit-slider-runnable-track {
  background: linear-gradient(90deg, #f00, #ff0, #0f0, #0ff, #00f, #f0f, #f00);
}

.color-picker-control__field input.color-picker-control__hue::-moz-range-track {
  background: linear-gradient(90deg, #f00, #ff0, #0f0, #0ff, #00f, #f0f, #f00);
}

.color-picker-control__hex input {
  width: 100%;
  min-width: 0;
  padding: 5px 7px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  font-size: 12px;
  font-family: ui-monospace, monospace;
}
</style>
