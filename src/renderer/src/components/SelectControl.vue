<script setup lang="ts" generic="T extends string | number | undefined">
import { computed, nextTick, onBeforeUnmount, ref, watch, useId } from 'vue'
import IconChevronDown from '~icons/mdi/chevron-down'
import { useFloatingPopover } from '@renderer/composables/useFloatingPopover'

interface SelectOption<T extends string | number | undefined> {
  label: string
  value: T
  group?: string
  disabled?: boolean
}

const props = withDefaults(
  defineProps<{
    modelValue: T
    options: readonly SelectOption<T>[]
    disabled?: boolean
    ariaLabel?: string
    placeholder?: string
    id?: string
  }>(),
  { disabled: false, placeholder: '' }
)

const emit = defineEmits<{ 'update:modelValue': [value: T] }>()
const generatedId = useId()
const root = ref<HTMLDivElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const panel = ref<HTMLDivElement | null>(null)
const open = ref(false)
const activeIndex = ref(-1)
const typeahead = ref('')
let typeaheadTimer: ReturnType<typeof setTimeout> | undefined
const closeForScroll = (): void => {
  open.value = false
  activeIndex.value = -1
}
const { style: panelStyle } = useFloatingPopover(open, trigger, panel, 'anchor', closeForScroll)

const sections = computed(() => {
  const result: { group?: string; options: { option: SelectOption<T>; index: number }[] }[] = []
  props.options.forEach((option, index) => {
    const previous = result[result.length - 1]
    if (!previous || previous.group !== option.group) {
      result.push({ group: option.group, options: [{ option, index }] })
    } else {
      previous.options.push({ option, index })
    }
  })
  return result
})

const selectedLabel = computed(
  () =>
    props.options.find((option) => Object.is(option.value, props.modelValue))?.label ||
    props.placeholder
)
const activeId = computed(() =>
  activeIndex.value < 0 ? undefined : `${generatedId}-option-${activeIndex.value}`
)

const firstEnabledIndex = (from: number, direction: 1 | -1): number => {
  if (!props.options.length) return -1
  for (let offset = 0; offset < props.options.length; offset++) {
    const index = (from + direction * offset + props.options.length * 2) % props.options.length
    if (!props.options[index].disabled) return index
  }
  return -1
}

const revealActive = (): void => {
  if (activeIndex.value < 0) return
  void nextTick(() => {
    panel.value
      ?.querySelector<HTMLElement>(`[data-option-index="${activeIndex.value}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  })
}

const openList = (): void => {
  if (props.disabled || !props.options.length) return
  const selectedIndex = props.options.findIndex(
    (option) => Object.is(option.value, props.modelValue) && !option.disabled
  )
  activeIndex.value = selectedIndex >= 0 ? selectedIndex : firstEnabledIndex(0, 1)
  open.value = true
  revealActive()
}

const closeList = (returnFocus = false): void => {
  open.value = false
  activeIndex.value = -1
  if (returnFocus) void nextTick(() => trigger.value?.focus())
}

const selectOption = (index: number): void => {
  const option = props.options[index]
  if (!option || option.disabled || props.disabled) return
  emit('update:modelValue', option.value)
  closeList(true)
}

const moveActive = (direction: 1 | -1): void => {
  activeIndex.value = firstEnabledIndex(activeIndex.value + direction, direction)
  revealActive()
}

const handleKeydown = (event: KeyboardEvent): void => {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    const direction = event.key === 'ArrowDown' ? 1 : -1
    if (!open.value) {
      const hasSelection = props.options.some(
        (option) => Object.is(option.value, props.modelValue) && !option.disabled
      )
      openList()
      if (hasSelection) moveActive(direction)
      else if (direction === -1) {
        activeIndex.value = firstEnabledIndex(props.options.length - 1, -1)
        revealActive()
      }
    } else moveActive(direction)
    return
  }
  if (event.key === 'Home' && open.value) {
    event.preventDefault()
    activeIndex.value = firstEnabledIndex(0, 1)
    revealActive()
    return
  }
  if (event.key === 'End' && open.value) {
    event.preventDefault()
    activeIndex.value = firstEnabledIndex(props.options.length - 1, -1)
    revealActive()
    return
  }
  if (event.key === 'Escape' && open.value) {
    event.preventDefault()
    closeList(true)
    return
  }
  if (event.key === 'Tab' && open.value) {
    closeList()
    return
  }
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    if (open.value) selectOption(activeIndex.value)
    else openList()
    return
  }
  if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
    typeahead.value += event.key.toLocaleLowerCase()
    if (typeaheadTimer) clearTimeout(typeaheadTimer)
    typeaheadTimer = setTimeout(() => (typeahead.value = ''), 700)
    const start = open.value ? activeIndex.value + 1 : 0
    const candidates = props.options.map((option, index) => ({ option, index }))
    const ordered = [...candidates.slice(start), ...candidates.slice(0, start)]
    const match = ordered.find(
      ({ option }) =>
        !option.disabled && option.label.toLocaleLowerCase().startsWith(typeahead.value)
    )
    if (match) {
      if (!open.value) {
        selectOption(match.index)
        return
      }
      activeIndex.value = match.index
      revealActive()
    }
  }
}

const onPointerDown = (event: PointerEvent): void => {
  const target = event.target
  if (target instanceof Node && !root.value?.contains(target) && !panel.value?.contains(target)) {
    closeList()
  }
}
const onFocusIn = (event: FocusEvent): void => {
  const target = event.target
  if (target instanceof Node && !root.value?.contains(target) && !panel.value?.contains(target)) {
    closeList()
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
  () => props.disabled,
  (disabled) => {
    if (disabled) closeList()
  }
)
watch(
  () => props.options,
  () => {
    if (!open.value) return
    const activeOption = props.options[activeIndex.value]
    if (!activeOption || activeOption.disabled) {
      activeIndex.value = firstEnabledIndex(0, 1)
    }
    if (activeIndex.value < 0) closeList()
    else revealActive()
  }
)
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onPointerDown, true)
  document.removeEventListener('focusin', onFocusIn, true)
  if (typeaheadTimer) clearTimeout(typeaheadTimer)
})
</script>

<template>
  <div ref="root" class="select-control">
    <button
      :id="id"
      ref="trigger"
      type="button"
      class="select-control__trigger"
      role="combobox"
      aria-haspopup="listbox"
      :aria-label="ariaLabel"
      :aria-expanded="open"
      :aria-controls="open ? `${generatedId}-listbox` : undefined"
      :aria-activedescendant="open ? activeId : undefined"
      :disabled="disabled"
      @click="open ? closeList() : openList()"
      @keydown="handleKeydown"
    >
      <span class="select-control__value" :class="{ 'is-placeholder': !selectedLabel }">
        {{ selectedLabel || placeholder }}
      </span>
      <IconChevronDown class="select-control__icon" aria-hidden="true" />
    </button>
    <Teleport to="body">
      <div
        v-if="open"
        :id="`${generatedId}-listbox`"
        ref="panel"
        class="select-control__popover"
        role="listbox"
        :aria-label="ariaLabel"
        :style="panelStyle"
      >
        <div
          v-for="(section, sectionIndex) in sections"
          :key="sectionIndex"
          :role="section.group ? 'group' : undefined"
          :aria-label="section.group"
        >
          <div v-if="section.group" class="select-control__group">{{ section.group }}</div>
          <button
            v-for="{ option, index } in section.options"
            :id="`${generatedId}-option-${index}`"
            :key="index"
            :data-option-index="index"
            type="button"
            class="select-control__option"
            :class="{ 'is-active': activeIndex === index }"
            role="option"
            :aria-selected="Object.is(option.value, modelValue)"
            :disabled="option.disabled"
            tabindex="-1"
            @mouseenter="!option.disabled && (activeIndex = index)"
            @mousedown.prevent
            @click="selectOption(index)"
          >
            <span>{{ option.label }}</span>
            <span
              v-if="Object.is(option.value, modelValue)"
              class="select-control__check"
              aria-hidden="true"
              >✓</span
            >
          </button>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style>
.select-control {
  display: inline-flex;
  min-width: 0;
  max-width: 100%;
  vertical-align: middle;
}

.select-control__trigger {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  min-width: 112px;
  min-height: 30px;
  padding: 5px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  font-size: 13px;
  line-height: 1.3;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--duration-fast),
    background-color var(--duration-fast);
}

.select-control__trigger:hover:not(:disabled) {
  border-color: var(--color-primary);
  background: var(--color-bg-tertiary);
}

.select-control__trigger:focus-visible {
  outline: none;
  border-color: var(--color-primary);
  background: var(--color-bg-tertiary);
}

.select-control__trigger:disabled {
  opacity: var(--opacity-disabled);
}

.select-control__value {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.select-control__value.is-placeholder {
  color: var(--color-text-secondary);
}

.select-control__icon {
  flex: none;
  width: 16px;
  height: 16px;
  color: var(--color-text-secondary);
}

.select-control__popover {
  z-index: 10000;
  overflow: auto;
  overscroll-behavior: contain;
  padding: 4px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-elevated);
  color: var(--color-text-primary);
  box-shadow: var(--shadow-elevated);
  scrollbar-width: thin;
}

.select-control__group {
  padding: 8px 9px 4px;
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 600;
}

.select-control__option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  min-height: 30px;
  padding: 6px 9px;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-primary);
  font-size: 13px;
  text-align: left;
  cursor: pointer;
}

.select-control__option:hover:not(:disabled),
.select-control__option.is-active:not(:disabled) {
  background: var(--color-selection);
}

.select-control__option[aria-selected='true'] {
  color: var(--color-primary);
  font-weight: 600;
}

.select-control__option:disabled {
  color: var(--color-text-tertiary);
  opacity: var(--opacity-disabled);
  cursor: not-allowed;
}

.select-control__check {
  flex: none;
  color: var(--color-primary);
}

.select-input {
  display: flex;
  width: 100%;
}
</style>
