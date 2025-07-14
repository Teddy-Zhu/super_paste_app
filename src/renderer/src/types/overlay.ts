/**
 * Overlay 组件相关的类型定义
 * 提供更严格的类型安全性
 *
 * 注意：部分类型已迁移到各自的 composable 文件中
 * 这里保留公共类型和向后兼容的类型定义
 */

import type { ClipboardItem, TagGroup } from '@renderer/api/types'
import { AnimationState } from '@renderer/composables/useAnimation'
import type { TabData } from '@renderer/composables/useScrollAndTags'
import type { Ref, ComputedRef } from 'vue'

// 重新导出动画相关类型
export { AnimationState, type AnimationConfig, type AnimationControls } from '@renderer/composables/useAnimation'

// 导入 AnimationControls 类型用于接口定义
import type { AnimationControls } from '@renderer/composables/useAnimation'

// 搜索动画配置类型
export interface SearchAnimationConfig {
  DURATION: number
  EASING: string
  SEARCH_WIDTH: string
  COLLAPSE_WIDTH: string
}

// 重新导出工具类型
export type {
  EventListenerCleanup,
  EventManager,
  TimerManager,
  DebounceFunction,
  ThrottleFunction
} from '@renderer/composables/useUtils'

// 重新导出性能优化类型
export type {
  BatchUpdateManager,
  VirtualScrollManager,
  PerformanceMonitor
} from '@renderer/composables/usePerformanceOptimization'

// 重新导出动画管理器类型
export type { AnimationManager } from '@renderer/composables/useAnimation'

// 重新导出搜索和导航管理器类型
export type { SearchAndNavigationManager } from '@renderer/composables/useSearchAndNavigation'

// 搜索管理器类型定义
export interface SearchManager {
  // 状态
  readonly searchText: Ref<string>
  readonly isSearching: Ref<boolean>
  readonly inputBuffer: Ref<string>

  // 内部状态修改方法
  _setSearching: (value: boolean) => void
  _clearInputBuffer: () => void
  _setSearchText: (value: string) => void

  // 方法
  debouncedSearch: (text: string) => void
  handleSearchInput: (event: Event) => void
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
  cleanup: () => void
  resetSearchState: () => void
}

// 滚动管理器类型定义
export interface ScrollManager {
  readonly isScrolling: Ref<boolean>
  scrollToSelectedItem: () => void
  handleWheel: (event: WheelEvent) => void
  ensureSelectedItemVisible: () => void
  initializeScrollbar: () => void
  destroyScrollbar: () => void
}

// 标签管理器类型定义
export interface TagManager {
  // 状态
  allTags: Ref<TagGroup[]>
  activeTagId: Ref<string>
  isTagSwitching: Ref<boolean>
  tagStats: Ref<Record<string, number>>
  tabs: ComputedRef<TabData[]>

  // 方法
  loadAllTags: () => Promise<void>
  loadTagStats: () => Promise<void>
  switchTag: (tagId: string) => Promise<void>
  reloadTags: () => Promise<void>
}

// 键盘导航管理器类型定义
export interface KeyboardNavigationManager {
  readonly isKeyboardNavigation: Ref<boolean>
  handleKeyNavigation: (event: KeyboardEvent) => void
  _setKeyboardNavigation: (value: boolean) => void
}

// 搜索动画管理器类型定义
export interface SearchAnimationManager {
  // 状态
  readonly isSearchVisible: Ref<boolean>
  readonly isSearchAnimating: Ref<boolean>
  readonly isOpeningSearch: Ref<boolean>
  showSearchContainer: ComputedRef<boolean>
  showTabsContainer: ComputedRef<boolean>

  // 内部状态修改方法
  _setSearchVisible: (value: boolean) => void
  _setSearchAnimating: (value: boolean) => void
  _setOpeningSearch: (value: boolean) => void

  // 动画方法
  createSearchExpandAnimation: () => AnimationControls
  createSearchCollapseAnimation: () => AnimationControls
  createTabsHideAnimation: () => AnimationControls
  createTabsShowAnimation: () => AnimationControls
  resetSearchStyles: () => void
  initializeStyles: () => void
}

// Overlay动画管理器类型定义
export interface OverlayAnimationManager {
  // 状态
  readonly currentAnimationState: Ref<AnimationState>
  readonly isSlideInComplete: Ref<boolean>

  // 方法
  setAnimationState: (state: AnimationState) => void
  canStartClosing: () => boolean
  isAnimating: () => boolean
  createSlideInAnimation: () => AnimationControls
  createSlideOutAnimation: () => AnimationControls
  resetState: () => void
}

// 重新导出滚动和标签管理器类型
export type { ScrollAndTagsManager } from '@renderer/composables/useScrollAndTags'

// 重新导出标签数据类型
export type { TabData } from '@renderer/composables/useScrollAndTags'

// 简化的组件类型定义
export interface OverlayProps {
  // 如果有 props 的话在这里定义
}

export interface OverlayEmits {
  // 如果有 emits 的话在这里定义
}

// 错误类型
export class OverlayError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly context?: Record<string, any>
  ) {
    super(message)
    this.name = 'OverlayError'
  }
}

// 错误代码枚举
export enum OverlayErrorCode {
  ANIMATION_FAILED = 'ANIMATION_FAILED',
  SEARCH_FAILED = 'SEARCH_FAILED',
  CLIPBOARD_LOAD_FAILED = 'CLIPBOARD_LOAD_FAILED',
  TAG_LOAD_FAILED = 'TAG_LOAD_FAILED',
  INVALID_STATE = 'INVALID_STATE'
}

// 工具类型
export type NonEmptyArray<T> = [T, ...T[]]
export type Optional<T, K extends keyof T> = Omit<T, K> & Partial<Pick<T, K>>
export type RequiredKeys<T> = {
  [K in keyof T]-?: {} extends Pick<T, K> ? never : K
}[keyof T]

// 类型守卫
export function isAnimationState(value: any): value is AnimationState {
  return Object.values(AnimationState).includes(value)
}

export function isTabData(value: any): value is TabData {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.id === 'string' &&
    typeof value.label === 'string' &&
    typeof value.active === 'boolean' &&
    typeof value.color === 'string' &&
    typeof value.count === 'number'
  )
}

export function isClipboardItem(value: any): value is ClipboardItem {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.id === 'string' &&
    typeof value.content === 'object' &&
    typeof value.primaryType === 'string' &&
    Array.isArray(value.availableFormats) &&
    typeof value.timestamp === 'number' &&
    typeof value.lastUsed === 'number' &&
    typeof value.parseType === 'string'
  )
}
