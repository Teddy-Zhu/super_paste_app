/**
 * 搜索功能 Composable
 * 管理搜索逻辑、输入缓冲和防抖搜索
 */

import { ref, nextTick, readonly } from 'vue'
import type { ClipboardHistoryQueryOptions } from '@renderer/api/types'
import type { SearchManager } from '@renderer/types/overlay'
import { useDebounce } from './usePerformanceOptimization'

export function useSearch(
  performSearchCallback: (text: string) => Promise<void>
): SearchManager {
  // 搜索状态
  const searchText = ref('')
  const isSearching = ref(false)

  // 输入缓冲区，用于在搜索框打开过程中缓存输入的字符
  const inputBuffer = ref('')

  /**
   * 防抖搜索（使用性能优化的防抖函数）
   */
  const [debouncedSearch, cancelSearch] = useDebounce(
    (text: string) => performSearchCallback(text),
    300
  )

  /**
   * 处理搜索输入
   */
  function handleSearchInput(event: Event) {
    const target = event.target as HTMLInputElement
    // 手动更新 searchText.value，因为模板使用的是 :value 而不是 v-model
    searchText.value = target.value
    // 触发防抖搜索
    debouncedSearch(target.value)
  }

  /**
   * 检查是否为可输入字符
   */
  function isInputCharacter(event: KeyboardEvent): boolean {
    // 排除功能键和修饰键
    if (event.ctrlKey || event.metaKey || event.altKey) {
      return false
    }

    // 排除特殊键
    const excludedKeys = [
      'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown',
      'Home', 'End', 'PageUp', 'PageDown',
      'Tab', 'Enter', 'Escape', 'Backspace', 'Delete',
      'Insert', 'F1', 'F2', 'F3', 'F4', 'F5', 'F6',
      'F7', 'F8', 'F9', 'F10', 'F11', 'F12',
      'CapsLock', 'NumLock', 'ScrollLock',
      'Shift', 'Control', 'Alt', 'Meta'
    ]

    if (excludedKeys.includes(event.key)) {
      return false
    }

    // 检查是否为可打印字符（长度为1的字符通常是可打印的）
    return event.key.length === 1
  }

  /**
   * 处理输入字符缓冲
   */
  function handleInputCharacter(
    event: KeyboardEvent,
    isSearchVisible: boolean,
    isOpeningSearch: boolean,
    isSearchAnimating: boolean,
    onToggleSearch: (withBuffer: boolean) => void
  ) {
    // 如果搜索框未完全打开（未打开或正在打开动画中），则缓存字符
    if (!isSearchVisible || isOpeningSearch || isSearchAnimating) {
      // 阻止默认行为，防止字符丢失
      event.preventDefault()

      // 将字符添加到缓冲区
      inputBuffer.value += event.key
      console.log('[前端] 字符已缓存到输入缓冲区:', inputBuffer.value)

      // 如果搜索框完全未打开，则开始打开
      if (!isSearchVisible && !isOpeningSearch) {
        onToggleSearch(true) // 传入 true 表示使用缓冲区
      }
    } else {
      // 搜索框已完全打开且动画完成，确保焦点在搜索框上
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput && document.activeElement !== searchInput) {
        // 如果搜索框没有焦点，给它焦点
        searchInput.focus()
        // 将当前字符添加到缓冲区，然后应用
        inputBuffer.value += event.key
        event.preventDefault()

        // 立即应用缓冲区内容
        if (inputBuffer.value) {
          searchText.value += inputBuffer.value
          inputBuffer.value = '' // 清空缓冲区

          // 更新输入框的值并将光标移动到末尾
          nextTick(() => {
            if (searchInput) {
              searchInput.value = searchText.value
              searchInput.setSelectionRange(searchText.value.length, searchText.value.length)
            }
          })
        }
      }
      // 如果搜索框已有焦点，不做任何处理，让默认行为处理输入
    }
  }

  /**
   * 应用缓冲区内容到搜索框
   */
  function applyInputBuffer() {
    if (inputBuffer.value) {
      // 追加缓冲区内容到现有搜索文本
      searchText.value += inputBuffer.value
      inputBuffer.value = '' // 清空缓冲区
      console.log('[前端] 缓冲区内容已应用到搜索框:', searchText.value)

      // 同步更新输入框的值
      nextTick(() => {
        const searchInput = document.querySelector('.search-input') as HTMLInputElement
        if (searchInput) {
          searchInput.value = searchText.value
          searchInput.setSelectionRange(searchText.value.length, searchText.value.length)
        }
      })

      // 触发搜索
      debouncedSearch(searchText.value)
    }
  }

  /**
   * 聚焦搜索框并设置光标位置
   */
  function focusSearchInput() {
    nextTick(() => {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput) {
        // 确保输入框的值与 searchText 同步
        searchInput.value = searchText.value
        searchInput.focus()
        // 将光标移动到末尾
        if (searchText.value) {
          searchInput.setSelectionRange(searchText.value.length, searchText.value.length)
        }
      }
    })
  }

  /**
   * 清理搜索相关资源
   */
  function cleanup() {
    // 清理搜索定时器（防抖函数会自动清理）
    cancelSearch()
  }

  /**
   * 重置搜索状态
   */
  function resetSearchState() {
    searchText.value = ''
    inputBuffer.value = ''
    isSearching.value = false
    cancelSearch()

    // 同步清空输入框的值
    nextTick(() => {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      if (searchInput) {
        searchInput.value = ''
      }
    })
  }

  return {
    // 状态
    searchText: readonly(searchText),
    isSearching: readonly(isSearching),
    inputBuffer: readonly(inputBuffer),

    // 内部状态修改方法
    _setSearching: (value: boolean) => { isSearching.value = value },
    _clearInputBuffer: () => { inputBuffer.value = '' },
    _setSearchText: (value: string) => { searchText.value = value },

    // 方法
    debouncedSearch,
    handleSearchInput,
    isInputCharacter,
    handleInputCharacter,
    applyInputBuffer,
    focusSearchInput,
    cleanup,
    resetSearchState
  }
}
