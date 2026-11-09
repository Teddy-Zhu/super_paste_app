<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import IconChevronDown from '~icons/mdi/chevron-down'
import IconChevronUp from '~icons/mdi/chevron-up'
import { FORM_CONTROL_TEXT } from '@renderer/constants/form-controls'

const props = withDefaults(
  defineProps<{
    modelValue: number | undefined
    min?: number
    max?: number
    step?: number
    emptyValue?: number
    allowEmpty?: boolean
    disabled?: boolean
    required?: boolean
    id?: string
    ariaLabel?: string
    placeholder?: string
    inputMode?: 'numeric' | 'decimal'
    clampOnCommit?: boolean
    incrementLabel?: string
    decrementLabel?: string
  }>(),
  {
    step: 1,
    allowEmpty: false,
    disabled: false,
    required: false,
    inputMode: 'numeric',
    clampOnCommit: true,
    incrementLabel: FORM_CONTROL_TEXT.increment,
    decrementLabel: FORM_CONTROL_TEXT.decrement
  }
)

const emit = defineEmits<{ 'update:modelValue': [value: number | undefined] }>()
const input = ref<HTMLInputElement | null>(null)
const focused = ref(false)
const draft = ref(format(props.modelValue))
const invalid = ref(false)

function format(value: number | undefined): string {
  return value === undefined || !Number.isFinite(value) ? '' : String(value)
}

const currentDraftValue = computed(() => {
  const raw = draft.value.trim()
  if (!raw) return props.modelValue
  const parsed = Number(raw)
  return Number.isFinite(parsed) ? parsed : props.modelValue
})
const decrementDisabled = computed(
  () =>
    props.disabled ||
    (props.min !== undefined &&
      currentDraftValue.value !== undefined &&
      currentDraftValue.value <= props.min)
)
const incrementDisabled = computed(
  () =>
    props.disabled ||
    (props.max !== undefined &&
      currentDraftValue.value !== undefined &&
      currentDraftValue.value >= props.max)
)

watch(
  () => props.modelValue,
  (value) => {
    if (!focused.value && !invalid.value) draft.value = format(value)
  }
)

const clamp = (value: number): number => {
  let bounded = value
  if (props.min !== undefined) bounded = Math.max(props.min, bounded)
  if (props.max !== undefined) bounded = Math.min(props.max, bounded)
  return bounded
}

const restore = (): void => {
  draft.value = format(props.modelValue)
  invalid.value = false
  input.value?.setCustomValidity('')
}

const commit = (): void => {
  if (props.disabled) return
  const raw = draft.value.trim()
  if (!raw) {
    invalid.value = false
    input.value?.setCustomValidity('')
    if (props.emptyValue !== undefined) {
      emit('update:modelValue', props.emptyValue)
      draft.value = format(props.emptyValue)
    } else if (props.allowEmpty) {
      emit('update:modelValue', undefined)
    } else if (!props.required) {
      restore()
    }
    return
  }

  const parsed = Number(raw)
  if (
    !Number.isFinite(parsed) ||
    (props.inputMode === 'numeric' && (!/^[+-]?\d+$/.test(raw) || !Number.isInteger(parsed)))
  ) {
    invalid.value = true
    input.value?.setCustomValidity(
      props.inputMode === 'numeric'
        ? FORM_CONTROL_TEXT.invalidNumber
        : FORM_CONTROL_TEXT.invalidDecimal
    )
    return
  }

  if (
    !props.clampOnCommit &&
    ((props.min !== undefined && parsed < props.min) ||
      (props.max !== undefined && parsed > props.max))
  ) {
    invalid.value = true
    input.value?.setCustomValidity(FORM_CONTROL_TEXT.numberOutOfRange)
    return
  }

  const value = props.clampOnCommit ? clamp(parsed) : parsed
  emit('update:modelValue', value)
  draft.value = format(value)
  invalid.value = false
  input.value?.setCustomValidity('')
}

const stepBy = (direction: 1 | -1): void => {
  if (props.disabled) return
  const draftValue = currentDraftValue.value ?? props.min ?? 0
  const current = props.inputMode === 'numeric' ? Math.round(draftValue) : draftValue
  const stepped = clamp(current + props.step * direction)
  const value = props.inputMode === 'numeric' ? Math.round(stepped) : stepped
  draft.value = format(value)
  invalid.value = false
  input.value?.setCustomValidity('')
  emit('update:modelValue', value)
  input.value?.focus()
}

const onInput = (event: Event): void => {
  draft.value = (event.target as HTMLInputElement).value
  invalid.value = false
  input.value?.setCustomValidity('')
}

const onBlur = (): void => {
  focused.value = false
  commit()
}

const onKeydown = (event: KeyboardEvent): void => {
  if (event.isComposing) return
  if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault()
    stepBy(event.key === 'ArrowUp' ? 1 : -1)
  } else if (event.key === 'Enter') {
    commit()
  }
}
</script>

<template>
  <div class="number-input" :class="{ 'is-disabled': disabled, 'is-invalid': invalid }">
    <input
      :id="id"
      ref="input"
      :value="draft"
      class="number-input__field"
      type="text"
      :inputmode="inputMode"
      autocomplete="off"
      role="spinbutton"
      :aria-label="ariaLabel"
      :aria-valuemin="min"
      :aria-valuemax="max"
      :aria-valuenow="currentDraftValue"
      :aria-invalid="invalid || undefined"
      :placeholder="placeholder"
      :disabled="disabled"
      :required="required"
      @focus="focused = true"
      @input="onInput"
      @keydown="onKeydown"
      @blur="onBlur"
    />
    <span class="number-input__steppers">
      <button
        type="button"
        class="number-input__step"
        :aria-label="incrementLabel"
        :disabled="incrementDisabled"
        @pointerdown.prevent
        @click="stepBy(1)"
      >
        <IconChevronUp aria-hidden="true" />
      </button>
      <button
        type="button"
        class="number-input__step"
        :aria-label="decrementLabel"
        :disabled="decrementDisabled"
        @pointerdown.prevent
        @click="stepBy(-1)"
      >
        <IconChevronDown aria-hidden="true" />
      </button>
    </span>
  </div>
</template>

<style>
.number-input {
  display: inline-flex;
  align-items: stretch;
  width: 80px;
  min-width: 64px;
  min-height: 28px;
  padding: 0;
  overflow: hidden;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  transition:
    border-color var(--duration-fast),
    background-color var(--duration-fast);
}

.number-input.small {
  width: 70px;
}

.number-input:focus-within:not(.is-invalid) {
  border-color: var(--color-primary);
}

.number-input:hover:not(.is-disabled):not(:focus-within):not(.is-invalid) {
  border-color: var(--color-primary);
  background: var(--color-bg-tertiary);
}

.number-input.is-disabled {
  background: var(--color-bg-secondary);
  box-shadow: none;
  opacity: var(--opacity-disabled);
}

.number-input:has(.number-input__field:disabled) {
  background: var(--color-bg-secondary);
  box-shadow: none;
  opacity: var(--opacity-disabled);
}

.number-input.is-invalid {
  border-color: var(--color-danger);
}

.number-input__field {
  flex: 1;
  width: 0;
  min-width: 0;
  padding: 3px 6px;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--color-text-primary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.number-input__field:focus-visible {
  outline: 0;
}

.number-input__field::placeholder {
  color: var(--color-text-secondary);
  font-size: 11px;
}

.number-input__steppers {
  display: flex;
  flex: none;
  flex-direction: column;
  width: 20px;
  border-left: 1px solid var(--color-border-light);
}

.number-input__step {
  display: grid;
  flex: 1;
  min-height: 0;
  place-items: center;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.number-input__step + .number-input__step {
  border-top: 1px solid var(--color-border-light);
}

.number-input__step:hover:not(:disabled) {
  background: var(--color-selection);
  color: var(--color-primary);
}

.number-input__step:focus-visible {
  outline: none;
  background: var(--color-selection);
  color: var(--color-primary);
}

.number-input__step:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.number-input__step svg {
  width: 12px;
  height: 12px;
}
</style>
