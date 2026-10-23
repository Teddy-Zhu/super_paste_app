/** 搜索输入、筛选结果与历史键盘导航。 */
import { readonly, ref, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import { useDebounce } from './useUtils'

interface SearchAndNavigationOptions {
  searchInput: Ref<HTMLInputElement | undefined>
  clipboardHistory: Ref<ClipboardItem[]>
  selectedIndex: Ref<number>
  isSearchVisible: Ref<boolean>
  isActive: () => boolean
  areResultsCurrent: () => boolean
  performSearch: (text: string) => Promise<boolean>
  onRestoreClipboard: (itemId: string) => Promise<void>
  onOpenSearch: () => Promise<void>
  onDismiss: () => Promise<void>
}

export interface SearchAndNavigationManager {
  readonly searchText: Readonly<Ref<string>>
  readonly isSearching: Readonly<Ref<boolean>>
  readonly isComposing: Readonly<Ref<boolean>>
  readonly isKeyboardNavigation: Readonly<Ref<boolean>>
  handleSearchInput: (event: Event) => void
  handleCompositionStart: () => void
  handleCompositionEnd: (event: CompositionEvent) => void
  focusSearchInput: () => void
  clearSearch: () => Promise<boolean>
  resetSearchState: () => void
  cancelPendingActions: () => void
  handleKeyNavigation: (event: KeyboardEvent) => void
  restoreSelectedItem: (index?: number) => Promise<void>
  setKeyboardNavigation: (value: boolean) => void
  cleanup: () => void
}

export function useSearchAndNavigation({
  searchInput,
  clipboardHistory,
  selectedIndex,
  isSearchVisible,
  isActive,
  areResultsCurrent,
  performSearch,
  onRestoreClipboard,
  onOpenSearch,
  onDismiss
}: SearchAndNavigationOptions): SearchAndNavigationManager {
  const searchText = ref('')
  const isSearching = ref(false)
  const isComposing = ref(false)
  const isKeyboardNavigation = ref(false)
  let searchRevision = 0
  let activeSearch: { revision: number; promise: Promise<boolean> } | undefined
  let historyActions = Promise.resolve()
  let restoreRevision: number | undefined
  let isPasting = false

  const ensureSearchCurrent = (): Promise<boolean> => {
    debouncedSearch.cancel()
    if (isComposing.value || !isActive()) return Promise.resolve(false)
    if (areResultsCurrent()) {
      isSearching.value = false
      return Promise.resolve(true)
    }
    if (activeSearch?.revision === searchRevision) return activeSearch.promise

    const revision = searchRevision
    isSearching.value = true
    const promise = performSearch(searchText.value)
      .then(
        (applied) =>
          applied && revision === searchRevision && !isComposing.value && areResultsCurrent()
      )
      .catch((error) => {
        console.error('[搜索框] 筛选失败:', error)
        return false
      })
      .finally(() => {
        if (activeSearch?.revision === revision) activeSearch = undefined
        if (revision === searchRevision) isSearching.value = false
      })
    activeSearch = { revision, promise }
    return promise
  }

  const debouncedSearch = useDebounce(() => ensureSearchCurrent(), 300)

  const focusSearchInput = (): void => {
    searchInput.value?.focus({ preventScroll: true })
  }

  const openSearch = (): void => {
    if (!isSearchVisible.value) {
      onOpenSearch().catch((error) => console.error('[搜索框] 展开失败:', error))
    }
  }

  const updateSearchText = (text: string): void => {
    if (text === searchText.value) return
    const filterChanged = text.trim() !== searchText.value.trim()
    searchText.value = text
    cancelPendingActions()
    if (filterChanged) {
      selectedIndex.value = 0
      isKeyboardNavigation.value = true
    }
  }

  const scheduleSearch = (): void => {
    if (isComposing.value) return
    isSearching.value = !areResultsCurrent()
    if (isSearching.value) debouncedSearch()
  }

  const handleSearchInput = (event: Event): void => {
    if (!isActive()) return
    updateSearchText((event.target as HTMLInputElement).value)
    openSearch()
    if (!(event as InputEvent).isComposing) scheduleSearch()
  }

  const handleCompositionStart = (): void => {
    if (!isActive()) return
    isComposing.value = true
    cancelPendingActions()
    openSearch()
  }

  const handleCompositionEnd = (event: CompositionEvent): void => {
    if (!isActive()) return
    isComposing.value = false
    updateSearchText((event.target as HTMLInputElement).value)
    scheduleSearch()
  }

  const cancelPendingActions = (): void => {
    ++searchRevision
    debouncedSearch.cancel()
    activeSearch = undefined
    historyActions = Promise.resolve()
    isSearching.value = false
  }

  const clearSearch = (): Promise<boolean> => {
    cancelPendingActions()
    updateSearchText('')
    return ensureSearchCurrent()
  }

  const resetSearchState = (): void => {
    cancelPendingActions()
    searchText.value = ''
    isComposing.value = false
    isKeyboardNavigation.value = false
  }

  // 等待同一筛选的结果后依次执行方向键和确认；输入变化会取消排队的旧操作。
  const queueHistoryAction = (action: () => void | Promise<void>): Promise<void> => {
    const revision = searchRevision
    historyActions = historyActions
      .then(async () => {
        if (revision !== searchRevision || !(await ensureSearchCurrent())) return
        if (revision !== searchRevision || !isActive()) return
        await action()
      })
      .catch((error) => console.error('[剪贴板] 键盘操作失败:', error))
    return historyActions
  }

  const restoreSelectedItem = async (index?: number): Promise<void> => {
    if (isComposing.value || isPasting || restoreRevision === searchRevision) return
    const revision = searchRevision
    restoreRevision = revision
    try {
      await queueHistoryAction(async () => {
        const item = clipboardHistory.value[index ?? selectedIndex.value]
        if (!item) return
        isPasting = true
        try {
          await onRestoreClipboard(item.id)
        } finally {
          isPasting = false
        }
      })
    } finally {
      if (restoreRevision === revision) restoreRevision = undefined
    }
  }

  const handleKeyNavigation = (event: KeyboardEvent): void => {
    if (
      event.defaultPrevented ||
      event.isComposing ||
      event.keyCode === 229 ||
      isComposing.value ||
      !isActive()
    )
      return

    const input = searchInput.value
    const inputFocused = document.activeElement === input
    const target = event.target
    if (
      target instanceof HTMLElement &&
      target !== input &&
      (target.matches('input, textarea, select') || target.isContentEditable)
    )
      return

    const isCtrlOrCmd = event.ctrlKey || event.metaKey
    if (inputFocused && event.shiftKey && event.key.startsWith('Arrow')) return

    switch (event.key) {
      case 'ArrowLeft':
      case 'ArrowRight':
      case 'ArrowUp':
      case 'ArrowDown':
      case 'Home':
      case 'End': {
        if ((event.key === 'Home' || event.key === 'End') && (inputFocused || !isCtrlOrCmd)) return
        event.preventDefault()
        if (isPasting || restoreRevision === searchRevision) return
        queueHistoryAction(() => {
          const maxIndex = clipboardHistory.value.length - 1
          if (maxIndex < 0) return
          isKeyboardNavigation.value = true
          switch (event.key) {
            case 'ArrowLeft':
              selectedIndex.value = Math.max(0, selectedIndex.value - (isCtrlOrCmd ? 5 : 1))
              break
            case 'ArrowRight':
              selectedIndex.value = Math.min(maxIndex, selectedIndex.value + (isCtrlOrCmd ? 5 : 1))
              break
            case 'ArrowUp':
              selectedIndex.value = isCtrlOrCmd ? 0 : Math.max(0, selectedIndex.value - 1)
              break
            case 'ArrowDown':
              selectedIndex.value = isCtrlOrCmd
                ? maxIndex
                : Math.min(maxIndex, selectedIndex.value + 1)
              break
            case 'Home':
              selectedIndex.value = 0
              break
            case 'End':
              selectedIndex.value = maxIndex
              break
          }
        })
        break
      }
      case 'Enter':
        event.preventDefault()
        if (!event.repeat) restoreSelectedItem()
        break
      case 'Escape':
        event.preventDefault()
        onDismiss().catch((error) => console.error('[搜索框] 退出失败:', error))
        break
      default:
        // 点击工具栏后继续键入时，只补入当前字符，其余编辑交给原生输入框。
        if (!inputFocused && input && !event.ctrlKey && !event.metaKey && !event.altKey) {
          if (Array.from(event.key).length === 1) {
            event.preventDefault()
            focusSearchInput()
            input.setRangeText(
              event.key,
              input.selectionStart ?? input.value.length,
              input.selectionEnd ?? input.value.length,
              'end'
            )
            updateSearchText(input.value)
            openSearch()
            scheduleSearch()
          }
        }
        break
    }
  }

  return {
    searchText: readonly(searchText),
    isSearching: readonly(isSearching),
    isComposing: readonly(isComposing),
    isKeyboardNavigation: readonly(isKeyboardNavigation),
    handleSearchInput,
    handleCompositionStart,
    handleCompositionEnd,
    focusSearchInput,
    clearSearch,
    resetSearchState,
    cancelPendingActions,
    handleKeyNavigation,
    restoreSelectedItem,
    setKeyboardNavigation: (value) => {
      isKeyboardNavigation.value = value
    },
    cleanup: cancelPendingActions
  }
}
