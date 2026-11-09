<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import IconCalendar from '~icons/mdi/calendar-month-outline'
import IconChevronLeft from '~icons/mdi/chevron-left'
import IconChevronRight from '~icons/mdi/chevron-right'
import { useFloatingPopover } from '@renderer/composables/useFloatingPopover'
import { FORM_CONTROL_TEXT } from '@renderer/constants/form-controls'

const props = withDefaults(
  defineProps<{
    modelValue: string
    disabled?: boolean
    ariaLabel?: string
    placeholder?: string
  }>(),
  { disabled: false, placeholder: FORM_CONTROL_TEXT.dateFormatPlaceholder }
)

const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const id = useId()
const anchor = ref<HTMLDivElement | null>(null)
const dateInput = ref<HTMLInputElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const panel = ref<HTMLElement | null>(null)
const open = ref(false)
const today = ref(new Date())
const dateFocused = ref(false)
const draft = ref(props.modelValue)
const invalid = ref(false)
let disposed = false

// setFullYear avoids JavaScript's 1900 offset for years below 100 and keeps local calendar dates.
const localDate = (year: number, monthIndex: number, day: number): Date => {
  const date = new Date(0)
  date.setFullYear(year, monthIndex, day)
  date.setHours(0, 0, 0, 0)
  return date
}

const parseDate = (value: string): Date | undefined => {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) return undefined
  const year = Number(match[1])
  if (year < 1 || year > 9999) return undefined
  const date = localDate(year, Number(match[2]) - 1, Number(match[3]))
  return date.getFullYear() === Number(match[1]) &&
    date.getMonth() === Number(match[2]) - 1 &&
    date.getDate() === Number(match[3])
    ? date
    : undefined
}
const toISODate = (date: Date): string =>
  `${String(date.getFullYear()).padStart(4, '0')}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
const selectedDate = computed(() => parseDate(props.modelValue))
const initialMonth = selectedDate.value || today.value
const month = ref(localDate(initialMonth.getFullYear(), initialMonth.getMonth(), 1))
const focusedDate = ref(selectedDate.value || today.value)
const closeForScroll = (): void => {
  open.value = false
}
const { style: panelStyle } = useFloatingPopover(open, anchor, panel, 286, closeForScroll)

const monthLabel = computed(() =>
  new Intl.DateTimeFormat('zh-CN', { year: 'numeric', month: 'long' }).format(month.value)
)
const calendarCells = computed(() => {
  const year = month.value.getFullYear()
  const monthIndex = month.value.getMonth()
  const startOffset = (localDate(year, monthIndex, 1).getDay() + 6) % 7
  const dayCount = localDate(year, monthIndex + 1, 0).getDate()
  return Array.from({ length: 42 }, (_, index) => {
    const day = index - startOffset + 1
    if (day < 1 || day > dayCount) return undefined
    return localDate(year, monthIndex, day)
  })
})
const calendarRows = computed(() =>
  Array.from({ length: 6 }, (_, index) => calendarCells.value.slice(index * 7, index * 7 + 7))
)

const focusCalendarDay = (date: Date): void => {
  void nextTick(() => {
    if (disposed || !open.value) return
    const button = panel.value?.querySelector<HTMLElement>(`[data-date="${toISODate(date)}"]`)
    button?.focus({ preventScroll: true })
    button?.scrollIntoView({ block: 'nearest' })
  })
}

const focusDate = (date: Date): void => {
  if (date.getFullYear() < 1 || date.getFullYear() > 9999) return
  focusedDate.value = date
  month.value = localDate(date.getFullYear(), date.getMonth(), 1)
  focusCalendarDay(date)
}

const openCalendar = (): void => {
  if (props.disabled) return
  commitDraft()
  today.value = new Date()
  const baseDate = parseDate(draft.value) || selectedDate.value || today.value
  month.value = localDate(baseDate.getFullYear(), baseDate.getMonth(), 1)
  focusedDate.value = baseDate
  open.value = true
  focusCalendarDay(baseDate)
}

const closeCalendar = (returnFocus = false): void => {
  open.value = false
  if (returnFocus)
    void nextTick(() => {
      if (!disposed && !props.disabled) dateInput.value?.focus({ preventScroll: true })
    })
}

const chooseDate = (date: Date): void => {
  if (props.disabled) return
  const value = toISODate(date)
  draft.value = value
  invalid.value = false
  dateInput.value?.setCustomValidity('')
  emit('update:modelValue', value)
  closeCalendar(true)
}

const clearDate = (): void => {
  if (props.disabled) return
  draft.value = ''
  invalid.value = false
  dateInput.value?.setCustomValidity('')
  emit('update:modelValue', '')
  closeCalendar(true)
}

const commitDraft = (): void => {
  if (props.disabled) return
  const value = draft.value.trim()
  invalid.value = value !== '' && !parseDate(value)
  dateInput.value?.setCustomValidity(invalid.value ? FORM_CONTROL_TEXT.invalidDate : '')
  if (invalid.value) return
  draft.value = value
  if (value !== props.modelValue) emit('update:modelValue', value)
}

const onDateInput = (event: Event): void => {
  draft.value = (event.target as HTMLInputElement).value
  invalid.value = false
  dateInput.value?.setCustomValidity('')
}

const onDateBlur = (): void => {
  dateFocused.value = false
  commitDraft()
}

const canShiftMonth = (amount: number): boolean => {
  const next = localDate(month.value.getFullYear(), month.value.getMonth() + amount, 1)
  return next.getFullYear() >= 1 && next.getFullYear() <= 9999
}

const shiftMonth = (amount: number, focusDay = false): void => {
  if (!canShiftMonth(amount)) return
  month.value = localDate(month.value.getFullYear(), month.value.getMonth() + amount, 1)
  const daysInMonth = localDate(month.value.getFullYear(), month.value.getMonth() + 1, 0).getDate()
  focusedDate.value = localDate(
    month.value.getFullYear(),
    month.value.getMonth(),
    Math.min(focusedDate.value.getDate(), daysInMonth)
  )
  if (focusDay) focusCalendarDay(focusedDate.value)
}

const handleDayKeydown = (event: KeyboardEvent, date: Date): void => {
  let next: Date | undefined
  if (event.key === 'ArrowLeft')
    next = localDate(date.getFullYear(), date.getMonth(), date.getDate() - 1)
  else if (event.key === 'ArrowRight')
    next = localDate(date.getFullYear(), date.getMonth(), date.getDate() + 1)
  else if (event.key === 'ArrowUp')
    next = localDate(date.getFullYear(), date.getMonth(), date.getDate() - 7)
  else if (event.key === 'ArrowDown')
    next = localDate(date.getFullYear(), date.getMonth(), date.getDate() + 7)
  else if (event.key === 'Home')
    next = localDate(
      date.getFullYear(),
      date.getMonth(),
      date.getDate() - ((date.getDay() + 6) % 7)
    )
  else if (event.key === 'End')
    next = localDate(
      date.getFullYear(),
      date.getMonth(),
      date.getDate() + (6 - ((date.getDay() + 6) % 7))
    )
  else if (event.key === 'PageUp') {
    event.preventDefault()
    shiftMonth(event.shiftKey ? -12 : -1, true)
    return
  } else if (event.key === 'PageDown') {
    event.preventDefault()
    shiftMonth(event.shiftKey ? 12 : 1, true)
    return
  }

  if (next) {
    event.preventDefault()
    focusDate(next)
  }
}

const onPointerDown = (event: PointerEvent): void => {
  const target = event.target
  if (
    target instanceof Node &&
    !dateInput.value?.contains(target) &&
    !trigger.value?.contains(target) &&
    !panel.value?.contains(target)
  ) {
    closeCalendar()
  }
}
const onFocusIn = (event: FocusEvent): void => {
  const target = event.target
  if (
    target instanceof Node &&
    !dateInput.value?.contains(target) &&
    !trigger.value?.contains(target) &&
    !panel.value?.contains(target)
  ) {
    closeCalendar()
  }
}
const onTriggerKeydown = (event: KeyboardEvent): void => {
  if (event.key === 'Escape') {
    event.preventDefault()
    if (open.value) closeCalendar(true)
    else {
      draft.value = props.modelValue
      invalid.value = false
      dateInput.value?.setCustomValidity('')
    }
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    openCalendar()
  } else if (event.key === 'Enter' && !event.isComposing) {
    commitDraft()
  }
}
const onPanelKeydown = (event: KeyboardEvent): void => {
  if (event.key === 'Escape') {
    event.preventDefault()
    closeCalendar(true)
  }
}

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
  () => props.modelValue,
  (value) => {
    const next = parseDate(value)
    if (!dateFocused.value) {
      draft.value = value
      invalid.value = false
      dateInput.value?.setCustomValidity('')
    }
    if (next && !open.value) {
      month.value = localDate(next.getFullYear(), next.getMonth(), 1)
      focusedDate.value = next
    }
  }
)
watch(
  () => props.disabled,
  (disabled) => {
    if (disabled) closeCalendar()
  }
)
onBeforeUnmount(() => {
  disposed = true
  document.removeEventListener('pointerdown', onPointerDown, true)
  document.removeEventListener('focusin', onFocusIn, true)
})
</script>

<template>
  <div ref="anchor" class="date-picker" :class="{ 'is-invalid': invalid }">
    <input
      ref="dateInput"
      class="date-picker__input"
      type="text"
      inputmode="text"
      maxlength="10"
      autocomplete="off"
      spellcheck="false"
      :value="draft"
      role="combobox"
      aria-haspopup="dialog"
      aria-autocomplete="none"
      :aria-label="ariaLabel || FORM_CONTROL_TEXT.chooseDate"
      :aria-expanded="open"
      :aria-controls="open ? `${id}-calendar` : undefined"
      :aria-invalid="invalid || undefined"
      :aria-describedby="invalid ? `${id}-error` : undefined"
      :placeholder="placeholder"
      :disabled="disabled"
      @focus="dateFocused = true"
      @input="onDateInput"
      @keydown="onTriggerKeydown"
      @blur="onDateBlur"
    />
    <button
      ref="trigger"
      type="button"
      class="date-picker__trigger"
      :aria-label="
        ariaLabel ? `${ariaLabel}，${FORM_CONTROL_TEXT.chooseDate}` : FORM_CONTROL_TEXT.chooseDate
      "
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="open ? `${id}-calendar` : undefined"
      :disabled="disabled"
      @click="open ? closeCalendar() : openCalendar()"
      @keydown="onTriggerKeydown"
    >
      <IconCalendar aria-hidden="true" />
    </button>
    <span v-if="invalid" :id="`${id}-error`" class="date-picker__error" role="alert">
      {{ FORM_CONTROL_TEXT.invalidDate }}
    </span>
    <Teleport to="body">
      <section
        v-if="open"
        :id="`${id}-calendar`"
        ref="panel"
        class="date-picker__panel"
        role="dialog"
        :aria-label="ariaLabel || FORM_CONTROL_TEXT.chooseDate"
        :style="panelStyle"
        @keydown="onPanelKeydown"
      >
        <header class="date-picker__header">
          <button
            type="button"
            :aria-label="FORM_CONTROL_TEXT.previousYear"
            :disabled="!canShiftMonth(-12)"
            @click="shiftMonth(-12)"
          >
            <IconChevronLeft class="is-double" aria-hidden="true" />
          </button>
          <button
            type="button"
            :aria-label="FORM_CONTROL_TEXT.previousMonth"
            :disabled="!canShiftMonth(-1)"
            @click="shiftMonth(-1)"
          >
            <IconChevronLeft aria-hidden="true" />
          </button>
          <strong>{{ monthLabel }}</strong>
          <button
            type="button"
            :aria-label="FORM_CONTROL_TEXT.nextMonth"
            :disabled="!canShiftMonth(1)"
            @click="shiftMonth(1)"
          >
            <IconChevronRight aria-hidden="true" />
          </button>
          <button
            type="button"
            :aria-label="FORM_CONTROL_TEXT.nextYear"
            :disabled="!canShiftMonth(12)"
            @click="shiftMonth(12)"
          >
            <IconChevronRight class="is-double" aria-hidden="true" />
          </button>
        </header>
        <div class="date-picker__grid" role="grid" :aria-label="monthLabel">
          <div class="date-picker__row" role="row">
            <span
              v-for="day in FORM_CONTROL_TEXT.weekDays"
              :key="day"
              class="date-picker__weekday"
              role="columnheader"
            >
              {{ day }}
            </span>
          </div>
          <div
            v-for="(row, index) in calendarRows"
            :key="index"
            class="date-picker__row"
            role="row"
          >
            <span
              v-for="(date, column) in row"
              :key="column"
              role="gridcell"
              :aria-selected="date ? toISODate(date) === modelValue : false"
            >
              <button
                v-if="date"
                type="button"
                class="date-picker__day"
                :class="{
                  'is-selected': toISODate(date) === modelValue,
                  'is-today': toISODate(date) === toISODate(today)
                }"
                :data-date="toISODate(date)"
                :aria-label="toISODate(date)"
                :aria-current="toISODate(date) === toISODate(today) ? 'date' : undefined"
                :tabindex="toISODate(date) === toISODate(focusedDate) ? 0 : -1"
                @click="chooseDate(date)"
                @keydown="handleDayKeydown($event, date)"
              >
                {{ date.getDate() }}
              </button>
            </span>
          </div>
        </div>
        <footer class="date-picker__footer">
          <button type="button" class="date-picker__today" @click="chooseDate(today)">
            {{ FORM_CONTROL_TEXT.today }}
          </button>
          <button
            type="button"
            class="date-picker__clear"
            :disabled="!modelValue"
            @click="clearDate"
          >
            {{ FORM_CONTROL_TEXT.clearDate }}
          </button>
        </footer>
      </section>
    </Teleport>
  </div>
</template>

<style>
.date-picker {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  min-width: 0;
  max-width: 100%;
  min-height: 30px;
  overflow: hidden;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  transition: border-color var(--duration-fast);
}

.date-picker:focus-within {
  border-color: var(--color-primary);
}

.date-picker.is-invalid {
  border-color: var(--color-danger);
}

.date-picker__error {
  flex-basis: 100%;
  max-width: 180px;
  padding: 4px 8px;
  color: var(--color-danger);
  font-size: 11px;
  white-space: normal;
}

.date-picker:has(:disabled) {
  opacity: var(--opacity-disabled);
}

.date-picker__trigger {
  display: grid;
  flex: none;
  width: 30px;
  height: 28px;
  place-items: center;
  padding: 0;
  border: 0;
  border-left: 1px solid var(--color-border-light);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.date-picker__trigger:hover:not(:disabled) {
  background: var(--color-selection);
  color: var(--color-primary);
}

.date-picker__trigger:focus-visible {
  outline: none;
}

.date-picker__trigger:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.date-picker__input {
  flex: 1;
  width: 124px;
  min-width: 92px;
  max-width: 100%;
  padding: 5px 8px;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--color-text-primary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.date-picker__input:focus-visible {
  outline: none;
}

.date-picker__trigger svg {
  width: 16px;
  height: 16px;
  color: var(--color-text-secondary);
}

.date-picker__panel {
  z-index: 10000;
  overflow: auto;
  padding: 10px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-elevated);
}

.date-picker__header {
  display: grid;
  grid-template-columns: 28px 28px minmax(0, 1fr) 28px 28px;
  align-items: center;
  gap: 3px;
  margin-bottom: 8px;
}

.date-picker__header strong {
  text-align: center;
  font-size: 13px;
}

.date-picker__header button,
.date-picker__footer button {
  display: inline-grid;
  min-width: 28px;
  min-height: 28px;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.date-picker__header button:hover:not(:disabled),
.date-picker__footer button:hover:not(:disabled) {
  background: var(--color-selection);
  color: var(--color-primary);
}

.date-picker__header button:focus-visible,
.date-picker__footer button:focus-visible {
  outline: none;
  background: var(--color-selection);
  color: var(--color-primary);
}

.date-picker__header svg {
  width: 16px;
  height: 16px;
}

.date-picker__header .is-double {
  opacity: 0.78;
}

.date-picker__grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: center;
}

.date-picker__row {
  display: grid;
  grid-template-columns: repeat(7, minmax(0, 1fr));
  gap: 2px;
  text-align: center;
}

.date-picker__row > span {
  display: grid;
  min-width: 0;
  min-height: 30px;
  place-items: center;
}

.date-picker__weekday {
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.date-picker__day {
  width: 30px;
  height: 30px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-primary);
  font-size: 12px;
  cursor: pointer;
}

.date-picker__day:hover,
.date-picker__day:focus-visible {
  outline: none;
  background: var(--color-selection);
  border-color: var(--color-primary);
}

.date-picker__day.is-selected {
  background: var(--color-primary);
  color: white;
}

.date-picker__day.is-today:not(.is-selected) {
  border-color: var(--color-primary);
  color: var(--color-primary);
  font-weight: 600;
}

.date-picker__footer {
  display: flex;
  justify-content: space-between;
  margin-top: 8px;
  border-top: 1px solid var(--color-border-light);
  padding-top: 6px;
}

.date-picker__footer button {
  padding: 0 8px;
  font-size: 12px;
}

.date-picker__footer .date-picker__today {
  color: var(--color-primary);
}

.date-picker__clear:disabled {
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}
</style>
