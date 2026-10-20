/**
 * 搜索和导航管理 Composable
 * 合并搜索逻辑和键盘导航，提供统一的交互接口
 */

import { computed, nextTick, readonly, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import { useDebounce, useStateManager } from './useUtils'

export interface SearchState {
  text: string
  isSearching: boolean
  inputBuffer: string
}

export interface NavigationState {
  isKeyboardNavigation: boolean
}

type ToggleSearchFn = (withBuffer?: boolean) => void | Promise<void>

export interface SearchAndNavigationManager {
  readonly searchText: Ref<string>
  readonly isSearching: Ref<boolean>
  readonly inputBuffer: Ref<string>
  readonly isKeyboardNavigation: Ref<boolean>

  handleSearchInput: (event: Event) => void
  debouncedSearch: (text: string) => void
  isInputCharacter: (event: KeyboardEvent) => boolean
  handleInputCharacter: (
    event: KeyboardEvent,
    isSearchVisible: boolean,
    isOpeningSearch: boolean,
    isSearchAnimating: boolean,
    onToggleSearch: ToggleSearchFn
  ) => void
  applyInputBuffer: () => void
  focusSearchInput: () => void
  resetSearchState: () => void

  handleKeyNavigation: (event: KeyboardEvent) => void
  setKeyboardNavigation: (value: boolean) => void

  cleanup: () => void
}

const EXCLUDED_KEYS = new Set([
  'ArrowLeft',
  'ArrowRight',
  'ArrowUp',
  'ArrowDown',
  'Home',
  'End',
  'PageUp',
  'PageDown',
  'Tab',
  'Enter',
  'Escape',
  'Backspace',
  'Delete',
  'Insert',
  'F1',
  'F2',
  'F3',
  'F4',
  'F5',
  'F6',
  'F7',
  'F8',
  'F9',
  'F10',
  'F11',
  'F12',
  'CapsLock',
  'NumLock',
  'ScrollLock',
  'Shift',
  'Control',
  'Alt',
  'Meta'
])

const SEARCH_INPUT_SELECTOR = '.search-input'

export function useSearchAndNavigation(
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  isSearchVisible: Ref<boolean>,
  performSearchCallback: (text: string) => Promise<void>,
  onRestoreClipboard: (itemId: string) => void,
  onCloseWindow: () => void,
  onToggleSearch: ToggleSearchFn
): SearchAndNavigationManager {
  const searchStateManager = useStateManager<SearchState>({
    text: '',
    isSearching: false,
    inputBuffer: ''
  })

  const navigationStateManager = useStateManager<NavigationState>({
    isKeyboardNavigation: false
  })

  const searchText = computed(() => searchStateManager.state.value.text)
  const isSearching = computed(() => searchStateManager.state.value.isSearching)
  const inputBuffer = computed(() => searchStateManager.state.value.inputBuffer)
  const isKeyboardNavigation = computed(
    () => navigationStateManager.state.value.isKeyboardNavigation
  )

  const debouncedSearchFn = useDebounce((text: string) => performSearchCallback(text), 300)

  const getSearchInput = (): HTMLInputElement | null => {
    return document.querySelector<HTMLInputElement>(SEARCH_INPUT_SELECTOR)
  }

  const syncInput = (value: string, focus = false): void => {
    nextTick(() => {
      const searchInput = getSearchInput()
      if (!searchInput) return

      searchInput.value = value
      if (focus) {
        searchInput.focus()
      }

      if (value) {
        searchInput.setSelectionRange(value.length, value.length)
      }
    })
  }

  const appendInputBuffer = (value: string): void => {
    searchStateManager.setState((prev) => ({
      ...prev,
      inputBuffer: prev.inputBuffer + value
    }))
  }

  const closeSearchOrWindow = (): void => {
    if (isSearchVisible.value) {
      onToggleSearch()
    } else {
      onCloseWindow()
    }
  }

  const isInputCharacter = (event: KeyboardEvent): boolean => {
    if (event.ctrlKey || event.metaKey || event.altKey) {
      return false
    }

    if (EXCLUDED_KEYS.has(event.key)) {
      return false
    }

    return event.key.length === 1
  }

  const handleSearchInput = (event: Event): void => {
    const target = event.target as HTMLInputElement
    const newText = target.value
    searchStateManager.setState((prev) => ({ ...prev, text: newText }))
    debouncedSearchFn(newText)
  }

  const applyInputBuffer = (): void => {
    const { text, inputBuffer } = searchStateManager.state.value
    if (!inputBuffer) {
      return
    }

    const mergedText = text + inputBuffer
    searchStateManager.setState((prev) => ({
      ...prev,
      text: mergedText,
      inputBuffer: ''
    }))

    syncInput(mergedText)
    debouncedSearchFn(mergedText)
  }

  const focusSearchInput = (): void => {
    const currentText = searchStateManager.state.value.text
    syncInput(currentText, true)
  }

  const handleInputCharacter = (
    event: KeyboardEvent,
    visible: boolean,
    opening: boolean,
    animating: boolean,
    toggleSearch: ToggleSearchFn
  ): void => {
    if (!visible || opening || animating) {
      event.preventDefault()
      appendInputBuffer(event.key)

      if (!visible && !opening) {
        toggleSearch(true)
      }
      return
    }

    const searchInput = getSearchInput()
    if (searchInput && document.activeElement !== searchInput) {
      event.preventDefault()
      searchInput.focus()
      appendInputBuffer(event.key)
      applyInputBuffer()
    }
  }

  const resetSearchState = (): void => {
    searchStateManager.resetState()
    debouncedSearchFn.cancel()
    syncInput('')
  }

  const setKeyboardNavigation = (value: boolean): void => {
    navigationStateManager.setState((prev) => ({
      ...prev,
      isKeyboardNavigation: value
    }))
  }

  const handleSearchInputNavigation = (event: KeyboardEvent): void => {
    switch (event.key) {
      case 'Escape':
        event.preventDefault()
        onToggleSearch()
        break
      case 'ArrowDown':
        if (clipboardHistory.value.length > 0) {
          event.preventDefault()
          getSearchInput()?.blur()
          selectedIndex.value = 0
          setKeyboardNavigation(true)
        }
        break
    }
  }

  const handleEmptyHistoryNavigation = (event: KeyboardEvent): void => {
    if (event.key === 'Escape') {
      event.preventDefault()
      closeSearchOrWindow()
      return
    }

    if (isInputCharacter(event)) {
      handleInputCharacter(event, isSearchVisible.value, false, false, onToggleSearch)
    }
  }

  const handleNavigationWithHistory = (event: KeyboardEvent, isCtrlOrCmd: boolean): void => {
    const maxIndex = clipboardHistory.value.length - 1

    switch (event.key) {
      case 'ArrowLeft':
        event.preventDefault()
        setKeyboardNavigation(true)
        selectedIndex.value = Math.max(0, selectedIndex.value - (isCtrlOrCmd ? 5 : 1))
        break
      case 'ArrowUp':
        event.preventDefault()
        setKeyboardNavigation(true)
        selectedIndex.value = isCtrlOrCmd ? 0 : Math.max(0, selectedIndex.value - 1)
        break
      case 'ArrowDown':
        event.preventDefault()
        setKeyboardNavigation(true)
        selectedIndex.value = isCtrlOrCmd ? maxIndex : Math.min(maxIndex, selectedIndex.value + 1)
        break
      case 'ArrowRight':
        event.preventDefault()
        setKeyboardNavigation(true)
        selectedIndex.value = Math.min(maxIndex, selectedIndex.value + (isCtrlOrCmd ? 5 : 1))
        break
      case 'Home':
        if (isCtrlOrCmd) {
          event.preventDefault()
          setKeyboardNavigation(true)
          selectedIndex.value = 0
        }
        break
      case 'End':
        if (isCtrlOrCmd) {
          event.preventDefault()
          setKeyboardNavigation(true)
          selectedIndex.value = maxIndex
        }
        break
      case 'Enter':
        event.preventDefault()
        if (clipboardHistory.value[selectedIndex.value]) {
          onRestoreClipboard(clipboardHistory.value[selectedIndex.value].id)
        }
        break
      case 'Escape':
        event.preventDefault()
        closeSearchOrWindow()
        break
      default:
        if (isInputCharacter(event)) {
          handleInputCharacter(event, isSearchVisible.value, false, false, onToggleSearch)
        }
        break
    }
  }

  const handleKeyNavigation = (event: KeyboardEvent): void => {
    const isCtrlOrCmd = event.ctrlKey || event.metaKey

    if (isSearchVisible.value) {
      const searchInput = getSearchInput()
      if (searchInput && document.activeElement === searchInput) {
        handleSearchInputNavigation(event)
        return
      }
    }

    if (clipboardHistory.value.length === 0) {
      handleEmptyHistoryNavigation(event)
      return
    }

    handleNavigationWithHistory(event, isCtrlOrCmd)
  }

  const cleanup = (): void => {
    debouncedSearchFn.cancel()
  }

  return {
    searchText: readonly(searchText),
    isSearching: readonly(isSearching),
    inputBuffer: readonly(inputBuffer),
    isKeyboardNavigation: readonly(isKeyboardNavigation),

    handleSearchInput,
    debouncedSearch: debouncedSearchFn,
    isInputCharacter,
    handleInputCharacter,
    applyInputBuffer,
    focusSearchInput,
    resetSearchState,

    handleKeyNavigation,
    setKeyboardNavigation,

    cleanup
  }
}
