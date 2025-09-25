/**
 * 键盘导航 Composable
 * 管理键盘事件处理和导航逻辑
 */

import { ref, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import type { KeyboardNavigationManager } from '@renderer/types/overlay'

export function useKeyboardNavigation(
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  onRestoreClipboard: (itemId: string) => void,
  onCloseWindow: () => void,
  onToggleSearch: () => void,
  onHandleInputCharacter: (event: KeyboardEvent) => void,
  isSearchVisible: Ref<boolean>
): KeyboardNavigationManager {
  // 标记是否由键盘导航触发的选择变化
  const isKeyboardNavigation = ref(false)

  /**
   * 处理键盘导航
   */
  function handleKeyNavigation(event: KeyboardEvent) {
    // 检查是否按住了 Ctrl (Windows/Linux) 或 Cmd (macOS)
    const isCtrlOrCmd = event.ctrlKey || event.metaKey

    // 如果搜索框已打开且获得焦点，只处理特定的导航键，其他键让搜索框处理
    if (isSearchVisible.value) {
      const searchInput = document.querySelector('.search-input') as HTMLInputElement
      const isSearchInputFocused = searchInput && document.activeElement === searchInput

      if (isSearchInputFocused) {
        // 搜索框有焦点时，只处理特定的导航键
        switch (event.key) {
          case 'Escape':
            event.preventDefault()
            console.log('esc1')
            onToggleSearch()
            break
          case 'ArrowDown':
            // 如果有剪贴板历史，从搜索框移动到第一个项目
            if (clipboardHistory.value.length > 0) {
              event.preventDefault()
              searchInput.blur()
              selectedIndex.value = 0
              isKeyboardNavigation.value = true
            }
            break
          default:
            // 其他键让搜索框自然处理，不干预
            return
        }
        return
      }
    }

    // 如果没有剪贴板历史，只处理输入字符和ESC键
    if (clipboardHistory.value.length === 0) {
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
          onHandleInputCharacter(event)
          break
      }
      return
    }

    // 有剪贴板历史时的完整键盘处理
    handleNavigationWithHistory(event, isCtrlOrCmd)
  }

  /**
   * 处理有历史记录时的键盘导航
   */
  function handleNavigationWithHistory(event: KeyboardEvent, isCtrlOrCmd: boolean) {
    switch (event.key) {
      case 'ArrowLeft':
        event.preventDefault()
        isKeyboardNavigation.value = true
        if (isCtrlOrCmd) {
          // Ctrl/Cmd + ArrowLeft：大幅度滚动（跳转5个位置）
          selectedIndex.value = Math.max(0, selectedIndex.value - 5)
        } else {
          // 普通方向键：单步移动
          selectedIndex.value = Math.max(0, selectedIndex.value - 1)
        }
        break
      case 'ArrowUp':
        event.preventDefault()
        isKeyboardNavigation.value = true
        if (isCtrlOrCmd) {
          // Ctrl/Cmd + ArrowUp：移动到第一个
          selectedIndex.value = 0
        } else {
          // 普通方向键：单步移动
          selectedIndex.value = Math.max(0, selectedIndex.value - 1)
        }
        break
      case 'ArrowDown':
        event.preventDefault()
        isKeyboardNavigation.value = true
        if (isCtrlOrCmd) {
          // Ctrl/Cmd + ArrowDown：移动到最后一个
          selectedIndex.value = clipboardHistory.value.length - 1
        } else {
          // 普通方向键：单步移动
          selectedIndex.value = Math.min(clipboardHistory.value.length - 1, selectedIndex.value + 1)
        }
        break
      case 'ArrowRight':
        event.preventDefault()
        isKeyboardNavigation.value = true
        if (isCtrlOrCmd) {
          // Ctrl/Cmd + ArrowRight：大幅度滚动（跳转5个位置）
          selectedIndex.value = Math.min(clipboardHistory.value.length - 1, selectedIndex.value + 5)
        } else {
          // 普通方向键：单步移动
          selectedIndex.value = Math.min(clipboardHistory.value.length - 1, selectedIndex.value + 1)
        }
        break
      case 'Home':
        // Home 键：跳转到第一个项目
        if (isCtrlOrCmd) {
          event.preventDefault()
          isKeyboardNavigation.value = true
          selectedIndex.value = 0
        }
        break
      case 'End':
        // End 键：跳转到最后一个项目
        if (isCtrlOrCmd) {
          event.preventDefault()
          isKeyboardNavigation.value = true
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
        onHandleInputCharacter(event)
        break
    }
  }

  return {
    isKeyboardNavigation: readonly(isKeyboardNavigation),
    handleKeyNavigation,
    _setKeyboardNavigation: (value: boolean) => {
      isKeyboardNavigation.value = value
    }
  }
}
