/**
 * 搜索和导航管理 Composable
 * 合并搜索逻辑和键盘导航，提供统一的交互接口
 */

import { nextTick, computed, readonly, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import { useDebounce, useStateManager } from './useUtils'

// ==================== 类型定义 ====================

export interface SearchState {
  text: string
  isSearching: boolean
  inputBuffer: string
}

export interface NavigationState {
  isKeyboardNavigation: boolean
}

export interface SearchAndNavigationManager {
  // 搜索状态
  readonly searchText: Ref<string>
  readonly isSearching: Ref<boolean>
  readonly inputBuffer: Ref<string>

  // 导航状态
  readonly isKeyboardNavigation: Ref<boolean>

  // 搜索方法
  handleSearchInput: (event: Event) => void
  debouncedSearch: (text: string) => void
  isInputCharacter: (event: KeyboardEvent) => boolean
  handleInputCharacter: (
    event: KeyboardEvent,
    isSearchVisible: boolean,
    isOpeningSearch: boolean,
    isSearchAnimating: boolean,
    onToggleSearch: (withBuffer: boolean) => void
  ) => void
  applyInputBuffer: () => void
  focusSearchInput: () => void
  resetSearchState: () => void

  // 导航方法
  handleKeyNavigation: (event: KeyboardEvent) => void
  setKeyboardNavigation: (value: boolean) => void

  // 清理方法
  cleanup: () => void
}

// ==================== 常量定义 ====================

const EXCLUDED_KEYS = [
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
]

// ==================== 主要函数 ====================

export function useSearchAndNavigation(
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  isSearchVisible: Ref<boolean>,
  performSearchCallback: (text: string) => Promise<void>,
  onRestoreClipboard: (itemId: string) => void,
  onCloseWindow: () => void,
  onToggleSearch: () => void
): SearchAndNavigationManager {
  // ==================== 状态管理 ====================

  const searchStateManager = useStateManager<SearchState>({
    text: '',
    isSearching: false,
    inputBuffer: ''
  })

  const navigationStateManager = useStateManager<NavigationState>({
    isKeyboardNavigation: false
  })

  // 提取状态引用
  const searchText = computed(() => searchStateManager.state.value.text)
  const isSearching = computed(() => searchStateManager.state.value.isSearching)
  const inputBuffer = computed(() => searchStateManager.state.value.inputBuffer)
  const isKeyboardNavigation = computed(
    () => navigationStateManager.state.value.isKeyboardNavigation
  )

  // ==================== 搜索功能 ====================

  // 防抖搜索
  const debouncedSearchFn = useDebounce((text: string) => performSearchCallback(text), 300)

  const handleSearchInput = (event: Event) => {
    const target = event.target as HTMLInputElement
    const newText = target.value

    searchStateManager.setState((prev) => ({ ...prev, text: newText }))
    debouncedSearchFn(newText)
  }

  const isInputCharacter = (event: KeyboardEvent): boolean => {
    // 排除功能键和修饰键
    if (event.ctrlKey || event.metaKey || event.altKey) {
      return false
    }

    // 排除特殊键
    if (EXCLUDED_KEYS.includes(event.key)) {
      return false
    }

    // 检查是否为可打印字符
    return event.key.length === 1
  }

  const handleInputCharacter = (
    event: KeyboardEvent,
    isSearchVisible: boolean,
    isOpeningSearch: boolean,
    isSearchAnimating: boolean,
    onToggleSearch: (withBuffer: boolean) => void
  ) => {
    // 如果搜索框未完全打开，则缓存字符
    if (!isSearchVisible || isOpeningSearch || isSearchAnimating) {
      event.preventDefault()

      // 将字符添加到缓冲区
      searchStateManager.setState((prev) => ({
        ...prev,
        inputBuffer: prev.inputBuffer + event.key
      }))

      console.log('[搜索导航] 字符已缓存:', searchStateManager.state.value.inputBuffer)

      // 如果搜索框完全未打开，则开始打开
      if (!isSearchVisible && !isOpeningSearch) {
        onToggleSearch(true)
      }
    } else {
      // 搜索框已完全打开，确保焦点在搜索框上
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput && document.activeElement !== searchInput) {
        searchInput.focus()

        // 将当前字符添加到缓冲区，然后应用
        searchStateManager.setState((prev) => ({
          ...prev,
          inputBuffer: prev.inputBuffer + event.key
        }))
        event.preventDefault()

        // 立即应用缓冲区内容
        applyInputBuffer()
      }
    }
  }

  const applyInputBuffer = () => {
    const currentBuffer = searchStateManager.state.value.inputBuffer
    if (currentBuffer) {
      const currentText = searchStateManager.state.value.text
      const newText = currentText + currentBuffer

      searchStateManager.setState((prev) => ({
        ...prev,
        text: newText,
        inputBuffer: ''
      }))

      console.log('[搜索导航] 缓冲区内容已应用:', newText)

      // 同步更新输入框
      nextTick(() => {
        const searchInput = document.querySelector('.search-input') as HTMLInputElement
        if (searchInput) {
          searchInput.value = newText
          searchInput.setSelectionRange(newText.length, newText.length)
        }
      })

      // 触发搜索
      debouncedSearchFn(newText)
    }
  }

  const focusSearchInput = () => {
    nextTick(() => {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput) {
        const currentText = searchStateManager.state.value.text
        searchInput.value = currentText
        searchInput.focus()
        if (currentText) {
          searchInput.setSelectionRange(currentText.length, currentText.length)
        }
      }
    })
  }

  const resetSearchState = () => {
    searchStateManager.resetState()
    debouncedSearchFn.cancel()

    // 同步清空输入框
    nextTick(() => {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput) {
        searchInput.value = ''
      }
    })
  }

  // ==================== 键盘导航功能 ====================

  const handleKeyNavigation = (event: KeyboardEvent) => {
    const isCtrlOrCmd = event.ctrlKey || event.metaKey

    // 如果搜索框已打开且获得焦点，只处理特定的导航键
    if (isSearchVisible.value) {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      const isSearchInputFocused = searchInput && document.activeElement === searchInput

      if (isSearchInputFocused) {
        handleSearchInputNavigation(event)
        return
      }
    }

    // 如果没有剪贴板历史，只处理输入字符和ESC键
    if (clipboardHistory.value.length === 0) {
      handleEmptyHistoryNavigation(event)
      return
    }

    // 有剪贴板历史时的完整键盘处理
    handleNavigationWithHistory(event, isCtrlOrCmd)
  }

  const handleSearchInputNavigation = (event: KeyboardEvent) => {
    switch (event.key) {
      case 'Escape':
        event.preventDefault()
        onToggleSearch()
        break
      case 'ArrowDown':
        // 从搜索框移动到第一个项目
        if (clipboardHistory.value.length > 0) {
          event.preventDefault()
          const searchInput = document.querySelector('.search-input') as HTMLInputElement
          if (searchInput) searchInput.blur()
          selectedIndex.value = 0
          setKeyboardNavigation(true)
        }
        break
    }
  }

  const handleEmptyHistoryNavigation = (event: KeyboardEvent) => {
    switch (event.key) {
      case 'Escape':
        event.preventDefault()
        if (isSearchVisible.value) {
          onToggleSearch()
        } else {
          onCloseWindow()
        }
        break
      default:
        // 检查是否为可输入字符
        if (isInputCharacter(event)) {
          handleInputCharacter(
            event,
            isSearchVisible.value,
            false, // isOpeningSearch
            false, // isSearchAnimating
            onToggleSearch
          )
        }
        break
    }
  }

  const handleNavigationWithHistory = (event: KeyboardEvent, isCtrlOrCmd: boolean) => {
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
        const maxIndex = clipboardHistory.value.length - 1
        selectedIndex.value = isCtrlOrCmd ? maxIndex : Math.min(maxIndex, selectedIndex.value + 1)
        break
      case 'ArrowRight':
        event.preventDefault()
        setKeyboardNavigation(true)
        const max = clipboardHistory.value.length - 1
        selectedIndex.value = Math.min(max, selectedIndex.value + (isCtrlOrCmd ? 5 : 1))
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
          selectedIndex.value = clipboardHistory.value.length - 1
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
        if (isSearchVisible.value) {
          onToggleSearch()
        } else {
          onCloseWindow()
        }
        break
      default:
        // 检查是否为可输入字符
        if (isInputCharacter(event)) {
          handleInputCharacter(
            event,
            isSearchVisible.value,
            false, // isOpeningSearch
            false, // isSearchAnimating
            onToggleSearch
          )
        }
        break
    }
  }

  const setKeyboardNavigation = (value: boolean) => {
    navigationStateManager.setState((prev) => ({
      ...prev,
      isKeyboardNavigation: value
    }))
  }

  // ==================== 清理方法 ====================

  const cleanup = () => {
    debouncedSearchFn.cancel()
  }

  // ==================== 返回接口 ====================

  return {
    // 搜索状态
    searchText: readonly(searchText),
    isSearching: readonly(isSearching),
    inputBuffer: readonly(inputBuffer),

    // 导航状态
    isKeyboardNavigation: readonly(isKeyboardNavigation),

    // 搜索方法
    handleSearchInput,
    debouncedSearch: debouncedSearchFn,
    isInputCharacter,
    handleInputCharacter,
    applyInputBuffer,
    focusSearchInput,
    resetSearchState,

    // 导航方法
    handleKeyNavigation,
    setKeyboardNavigation,

    // 清理方法
    cleanup
  }
}
