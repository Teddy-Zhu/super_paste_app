/**
 * Overlay 组件相关的类型定义
 * 提供更严格的类型安全性
 */

import type { Ref, ComputedRef } from 'vue'
import type { ClipboardItem, TagGroup } from '@renderer/api/types'

// 动画状态枚举
export enum AnimationState {
  IDLE = 'IDLE',
  SLIDING_IN = 'SLIDING_IN',
  ACTIVE = 'ACTIVE',
  SLIDING_OUT = 'SLIDING_OUT'
}

// 动画配置类型
export interface AnimationConfig {
  readonly DURATION: number
  readonly SLIDE_IN_EASING: string
  readonly SLIDE_OUT_EASING: string
}

// 搜索动画配置类型
export interface SearchAnimationConfig {
  readonly DURATION: number
  readonly EASING: string
  readonly SEARCH_WIDTH: string
  readonly COLLAPSE_WIDTH: string
}

// 动画函数返回类型
export interface AnimationControls {
  play: () => void
  playState: Ref<'idle' | 'running' | 'paused' | 'finished'>
}

// Overlay 动画管理器类型
export interface OverlayAnimationManager {
  readonly currentAnimationState: Ref<AnimationState>
  readonly isSlideInComplete: Ref<boolean>
  setAnimationState: (newState: AnimationState) => void
  canStartClosing: () => boolean
  isAnimating: () => boolean
  createSlideInAnimation: () => AnimationControls
  createSlideOutAnimation: () => AnimationControls
  resetState: () => void
}

// 搜索动画管理器类型
export interface SearchAnimationManager {
  readonly isSearchVisible: Ref<boolean>
  readonly isSearchAnimating: Ref<boolean>
  readonly isOpeningSearch: Ref<boolean>
  readonly showSearchContainer: ComputedRef<boolean>
  readonly showTabsContainer: ComputedRef<boolean>
  _setSearchVisible: (value: boolean) => void
  _setSearchAnimating: (value: boolean) => void
  _setOpeningSearch: (value: boolean) => void
  createSearchExpandAnimation: () => AnimationControls
  createSearchCollapseAnimation: () => AnimationControls
  createTabsHideAnimation: () => AnimationControls
  createTabsShowAnimation: () => AnimationControls
  resetSearchStyles: () => void
  initializeStyles: () => void
}

// 搜索管理器类型
export interface SearchManager {
  readonly searchText: Ref<string>
  readonly isSearching: Ref<boolean>
  readonly inputBuffer: Ref<string>
  _setSearching: (value: boolean) => void
  _clearInputBuffer: () => void
  _setSearchText: (value: string) => void
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

// 键盘导航管理器类型
export interface KeyboardNavigationManager {
  readonly isKeyboardNavigation: Ref<boolean>
  handleKeyNavigation: (event: KeyboardEvent) => void
  _setKeyboardNavigation: (value: boolean) => void
}

// 滚动管理器类型
export interface ScrollManager {
  readonly isScrolling: Ref<boolean>
  scrollToSelectedItem: () => void
  handleWheel: (event: WheelEvent) => void
  ensureSelectedItemVisible: () => void
}

// 标签数据类型
export interface TabData {
  id: string
  label: string
  active: boolean
  color: string
  count: number
}

// 标签管理器类型
export interface TagManager {
  readonly allTags: Ref<TagGroup[]>
  readonly activeTagId: Ref<string>
  readonly isTagSwitching: Ref<boolean>
  readonly tagStats: Ref<Record<string, number>>
  readonly tabs: ComputedRef<TabData[]>
  loadAllTags: () => Promise<void>
  loadTagStats: () => Promise<void>
  switchTag: (tagId: string) => Promise<void>
  reloadTags: () => Promise<void>
}

// 性能优化相关类型
export type DebouncedFunction<T extends (...args: any[]) => any> = T
export type ThrottledFunction<T extends (...args: any[]) => any> = T
export type CancelFunction = () => void

export interface EventListenerManager {
  addEventListener: (
    element: EventTarget,
    event: string,
    handler: EventListener,
    options?: AddEventListenerOptions
  ) => void
  removeEventListener: (
    element: EventTarget,
    event: string,
    handler: EventListener
  ) => void
  removeAllListeners: () => void
}

export interface MemoryLeakGuard {
  setTimeout: (callback: () => void, delay: number) => NodeJS.Timeout
  setInterval: (callback: () => void, delay: number) => NodeJS.Timeout
  requestAnimationFrame: (callback: FrameRequestCallback) => number
  clearTimeout: (timer: NodeJS.Timeout) => void
  clearInterval: (interval: NodeJS.Timeout) => void
  cancelAnimationFrame: (raf: number) => void
  cleanup: () => void
}

// 批量更新类型
export interface BatchUpdateManager<T> {
  addUpdate: (item: T) => void
  flush: () => void
}

// 组件 Props 类型
export interface OverlayProps {
  // 如果有 props 的话在这里定义
}

// 组件 Emits 类型
export interface OverlayEmits {
  // 如果有 emits 的话在这里定义
}

// 组件实例类型
export interface OverlayInstance {
  overlayContentRef: Ref<HTMLElement | undefined>
  clipboardScrollRef: Ref<HTMLElement | undefined>
  searchContainerRef: Ref<HTMLElement | undefined>
  tabsContainerRef: Ref<HTMLElement | undefined>
  clipboardHistory: Ref<ClipboardItem[]>
  selectedIndex: Ref<number>
  overlayAnimation: OverlayAnimationManager
  searchAnimation: SearchAnimationManager
  searchLogic: SearchManager
  keyboardNavigation: KeyboardNavigationManager
  scrollManagement: ScrollManager
  tagManagement: TagManager
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
