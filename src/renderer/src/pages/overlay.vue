<script setup lang="ts">
import { ref, nextTick, watch, onMounted, onUnmounted } from 'vue'
import { send, onSimple } from '@renderer/api'
import { clipboard } from '@renderer/api/renderer-api'
import type { ClipboardItem, ClipboardHistoryQueryOptions, AppInfo } from '@renderer/api/types'
import {
  OverlayError,
  OverlayErrorCode,
  isClipboardItem
} from '@renderer/types/overlay'

// 导入优化后的 composables
import { useAnimation, AnimationState } from '@renderer/composables/useAnimation'
import { useSearchAndNavigation } from '@renderer/composables/useSearchAndNavigation'
import { useScrollAndTags } from '@renderer/composables/useScrollAndTags'
import { useEventManager, useThrottle, withTimeout } from '@renderer/composables/useUtils'

// 导入组件
import ClipboardCard from '@renderer/components/ClipboardCard.vue'
import ContextMenu from '@renderer/components/ContextMenu.vue'

// 导入图标
import IconSettings from '~icons/mdi/cog'
import IconDelete from '~icons/mdi/delete'

// ==================== DOM 引用 ====================
const overlayContentRef = ref<HTMLElement>()
const clipboardScrollRef = ref<HTMLElement>()
const searchContainerRef = ref<HTMLElement>()
const tabsContainerRef = ref<HTMLElement>()

// ==================== 核心状态 ====================
const clipboardHistory = ref<ClipboardItem[]>([])
const clipboardApps = ref<Record<string, AppInfo>>({})
const selectedIndex = ref(0)

// ==================== 快捷选择状态 ====================
const isQuickSelectMode = ref(false)

// ==================== 下拉菜单状态 ====================
const isDropdownVisible = ref(false)
const dropdownMenuRef = ref<HTMLElement>()

// ==================== 事件管理器 ====================
const eventManager = useEventManager()

// ==================== 监听器状态管理 ====================
let isCloseListenerSetup = false
let removeCloseListener: (() => void) | null = null
let wheelHandler: ((event: WheelEvent) => void) | null = null

// ==================== 初始化 Composables ====================
const animationManager = useAnimation(
  overlayContentRef,
  searchContainerRef,
  tabsContainerRef
)

// ==================== 剪贴板历史加载 ====================

/**
 * 获取剪贴板历史记录（带类型验证和错误处理）
 */
const loadClipboardHistory = async (options?: ClipboardHistoryQueryOptions): Promise<void> => {
  try {
    const defaultOptions: ClipboardHistoryQueryOptions = {
      limit: 200,
      orderBy: 'last_used',
      orderDirection: 'DESC'
    }

    const queryOptions = { ...defaultOptions, ...options }
    const result = await clipboard.searchHistory(queryOptions)

    if (!Array.isArray(result.items)) {
      throw new OverlayError(
        '剪贴板历史数据格式无效：不是数组',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { receivedType: typeof result.items, queryOptions }
      )
    }

    const validHistory = result.items.filter(isClipboardItem)
    clipboardHistory.value = validHistory
    clipboardApps.value = result.apps

    if (selectedIndex.value >= validHistory.length) {
      selectedIndex.value = Math.max(0, validHistory.length - 1)
    }

    nextTick(() => {
      scrollAndTagsManager.initializeScrollbar()
      scrollAndTagsManager.scrollToSelectedItem()
    })

  } catch (error) {
    console.error('获取剪贴板历史失败:', error)
    clipboardHistory.value = []
    clipboardApps.value = {}
    selectedIndex.value = 0
  }
}

const throttledLoadClipboardHistory = useThrottle(
  () => {
    const currentTagIds = scrollAndTagsManager.activeTagId.value === 'all'
      ? []
      : [scrollAndTagsManager.activeTagId.value]
    const currentSearchText = searchAndNavManager.searchText.value || ''

    loadClipboardHistory({
      tagIds: currentTagIds,
      searchText: currentSearchText
    }).catch(console.error)
  },
  500
)

// ==================== 临时占位符，将在 searchAndNavManager 定义后重新定义 ====================
let scrollAndTagsManager: any

// ==================== 事件监听器管理 ====================

/**
 * 设置关闭事件监听器
 */
const setupCloseEventListeners = () => {
  if (isCloseListenerSetup) return

  if (removeCloseListener) {
    removeCloseListener()
    removeCloseListener = null
  }

  removeCloseListener = onSimple('trigger-close-window', handleCloseWindow)
  isCloseListenerSetup = true
}

// ==================== 核心功能函数 ====================

/**
 * 通用剪贴板还原函数
 */
const restoreClipboard = async (itemId: string, restoreMethod: 'restore' | 'restoreAsText'): Promise<void> => {
  try {
    if (!itemId || typeof itemId !== 'string') {
      throw new OverlayError(
        '无效的剪贴板项目ID',
        OverlayErrorCode.INVALID_STATE,
        { itemId, type: typeof itemId }
      )
    }

    const success = await clipboard[restoreMethod](itemId)

    if (success) {
      closeWindow()
    } else {
      throw new OverlayError(
        `剪贴板${restoreMethod === 'restoreAsText' ? '纯文本' : ''}还原操作失败`,
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { itemId }
      )
    }
  } catch (error) {
    console.error(`[前端] 还原剪贴板${restoreMethod === 'restoreAsText' ? '为纯文本' : ''}失败:`, error)
  }
}

/**
 * 还原剪贴板内容
 */
const restoreClipboardContent = (itemId: string) => restoreClipboard(itemId, 'restore')

/**
 * 还原剪贴板内容为纯文本
 */
const restoreClipboardContentAsText = (itemId: string) => restoreClipboard(itemId, 'restoreAsText')

/**
 * 删除剪贴板项目
 */
const deleteClipboardItem = async (itemId: string): Promise<void> => {
  try {
    if (!itemId || typeof itemId !== 'string') {
      throw new OverlayError(
        '无效的剪贴板项目ID',
        OverlayErrorCode.INVALID_STATE,
        { itemId, type: typeof itemId }
      )
    }

    // 删除项目后重新加载历史记录
    throttledLoadClipboardHistory()
  } catch (error) {
    console.error('[前端] 删除剪贴板项目失败:', error)
  }
}

/**
 * 切换下拉菜单显示状态
 */
const toggleDropdownMenu = (): void => {
  isDropdownVisible.value = !isDropdownVisible.value
}

/**
 * 隐藏下拉菜单
 */
const hideDropdownMenu = (): void => {
  isDropdownVisible.value = false
}

/**
 * 打开设置窗口
 */
const openSettings = (): void => {
  try {
    hideDropdownMenu()
    // 关闭当前覆盖层窗口
    closeWindow()
    // 发送消息到主进程打开设置窗口
    send('open-main-window')
  } catch (error) {
    console.error('[前端] 打开设置失败:', error)
  }
}

/**
 * 清空剪贴板历史
 */
const clearClipboardHistory = async (): Promise<void> => {
  try {
    hideDropdownMenu()
    // 添加确认对话框
    if (confirm('确定要清空所有剪贴板历史记录吗？此操作不可撤销。')) {
      await clipboard.clearHistory()
      // await loadClipboardHistory()
    }
  } catch (error) {
    console.error('[前端] 清空历史失败:', error)
  }
}

/**
 * 等待动画完成的通用函数
 */
const waitForAnimationFinish = (playState: any): Promise<void> => {
  return new Promise((resolve) => {
    if (playState.value === 'finished') {
      resolve()
      return
    }

    const stopWatching = watch(playState, (newState) => {
      if (newState === 'finished') {
        stopWatching()
        resolve()
      }
    })
  })
}

/**
 * 关闭窗口
 */
const closeWindow = async () => {
  if (!animationManager.canStartClosing()) return

  animationManager.setAnimationState(AnimationState.SLIDING_OUT)
  const { play: playSlideOut, playState } = animationManager.createSlideOutAnimation()

  playSlideOut()
  await waitForAnimationFinish(playState)

  animationManager.resetOverlayState()
  send('close-overlay-window')
}

/**
 * 切换搜索框显示状态
 */
const toggleSearch = async (withBuffer = false): Promise<void> => {
  try {
    if (animationManager.isSearchAnimating.value) return

    const isCurrentlyVisible = animationManager.isSearchVisible.value

    if (isCurrentlyVisible) {
      await hideSearchBox()
    } else {
      await showSearchBox(withBuffer)
    }
  } catch (error) {
    console.error('[前端] 搜索框切换失败:', error)
    animationManager.setSearchAnimating(false)
    animationManager.setOpeningSearch(false)
    throw new OverlayError(
      '搜索框切换动画失败',
      OverlayErrorCode.ANIMATION_FAILED,
      { withBuffer, originalError: error }
    )
  }
}

/**
 * 隐藏搜索框
 */
const hideSearchBox = async (): Promise<void> => {
  animationManager.setSearchAnimating(true)

  const { play: playSearchCollapse, playState: searchCollapseState } = animationManager.createSearchCollapseAnimation()
  const { play: playTabsShow, playState: tabsShowState } = animationManager.createTabsShowAnimation()

  playSearchCollapse()
  playTabsShow()

  await Promise.all([
    waitForAnimationFinish(searchCollapseState),
    waitForAnimationFinish(tabsShowState)
  ])

  animationManager.setSearchVisible(false)
  animationManager.setSearchAnimating(false)
}

/**
 * 显示搜索框
 */
const showSearchBox = async (withBuffer: boolean): Promise<void> => {
  animationManager.setOpeningSearch(true)
  animationManager.setSearchAnimating(true)
  animationManager.setSearchVisible(true)

  await nextTick()

  const { play: playTabsHide, playState: tabsHideState } = animationManager.createTabsHideAnimation()
  const { play: playSearchExpand, playState: searchExpandState } = animationManager.createSearchExpandAnimation()

  playTabsHide()
  playSearchExpand()

  await Promise.all([
    waitForAnimationFinish(tabsHideState),
    waitForAnimationFinish(searchExpandState)
  ])

  animationManager.setSearchAnimating(false)
  animationManager.setOpeningSearch(false)

  if (withBuffer) {
    searchAndNavManager.applyInputBuffer()
  }

  searchAndNavManager.focusSearchInput()
}

// ==================== 搜索回调函数 ====================
const performSearchCallback = async (text: string): Promise<void> => {
  try {
    const currentTagIds = scrollAndTagsManager.activeTagId.value === 'all'
      ? []
      : [scrollAndTagsManager.activeTagId.value]

    const options: ClipboardHistoryQueryOptions = {
      searchText: text.trim(),
      tagIds: currentTagIds,
      limit: 50,
      orderBy: 'last_used' as const,
      orderDirection: 'DESC' as const
    }

    await loadClipboardHistory(options)
  } catch (error) {
    console.error('[前端] 搜索执行失败:', error)
    throw new OverlayError(
      '搜索操作失败',
      OverlayErrorCode.SEARCH_FAILED,
      { searchText: text, originalError: error }
    )
  }
}

// ==================== 初始化搜索和导航管理 ====================
const searchAndNavManager = useSearchAndNavigation(
  clipboardHistory,
  selectedIndex,
  animationManager.isSearchVisible,
  performSearchCallback,
  restoreClipboardContent,
  closeWindow,
  toggleSearch
)

// ==================== 初始化滚动和标签管理 ====================
scrollAndTagsManager = useScrollAndTags(
  clipboardScrollRef,
  clipboardHistory,
  selectedIndex,
  animationManager.currentAnimationState,
  loadClipboardHistory,
  () => searchAndNavManager.searchText.value || ''
)

// ==================== 动画流程函数 ====================

/**
 * 触发滑入动画
 */
const triggerSlideInAnimation = async (): Promise<void> => {
  try {
    if (animationManager.isAnimating()) return

    // 重置搜索状态
    if (!searchAndNavManager.searchText.value.trim()) {
      animationManager.resetSearchStyles()
    }

    searchAndNavManager.resetSearchState()
    animationManager.setAnimationState(AnimationState.SLIDING_IN)

    const { play: playSlideIn, playState } = animationManager.createSlideInAnimation()
    playSlideIn()

    // 等待滑入动画完成（带超时）
    await withTimeout(
      waitForAnimationFinish(playState),
      5000,
      '滑入动画超时'
    )

    animationManager.setAnimationState(AnimationState.ACTIVE)
    setupCloseEventListeners()
    send('open-overlay-finished')

  } catch (error) {
    console.error('[前端] 滑入动画失败:', error)
    animationManager.resetOverlayState()
    throw new OverlayError(
      '滑入动画执行失败',
      OverlayErrorCode.ANIMATION_FAILED,
      { originalError: error }
    )
  }
}

/**
 * 处理主进程发送的关闭窗口事件
 */
const handleCloseWindow = async (force: boolean = false): Promise<void> => {
  try {
    if (animationManager.currentAnimationState.value !== AnimationState.ACTIVE) return

    if (!force) {
      const isSearchVisible = animationManager.isSearchVisible.value
      const hasSearchText = searchAndNavManager.searchText.value.trim()

      if (isSearchVisible && hasSearchText) {
        searchAndNavManager.resetSearchState()
        await performSearchCallback('')
        return
      }

      if (isSearchVisible && !hasSearchText) {
        await toggleSearch()
        return
      }
    }

    await closeWindow()
  } catch (error) {
    console.error('[前端] 处理关闭窗口事件失败:', error)
    try {
      await closeWindow()
    } catch (closeError) {
      send('close-overlay-window')
    }
  }
}

// ==================== 快捷选择功能 ====================

/**
 * 处理键盘按下事件
 */
const handleKeyboardEvent = (event: KeyboardEvent) => {
  // 检查是否按下了 Ctrl (Windows/Linux) 或 Cmd (macOS)
  const isCtrlOrCmd = event.ctrlKey || event.metaKey

  // 快捷选择功能：Ctrl/Cmd + 数字键
  if (isCtrlOrCmd) {
    const keyCode = event.key
    const numberKey = parseInt(keyCode)

    if (!isNaN(numberKey) && numberKey >= 1 && numberKey <= 9) {
      // 计算对应的索引（从0开始）
      const targetIndex = numberKey - 1

      // 检查索引是否有效
      if (targetIndex < clipboardHistory.value.length) {
        const targetItem = clipboardHistory.value[targetIndex]
        restoreClipboardContent(targetItem.id)
        event.preventDefault()
        return
      }
    }

    // 进入快捷选择模式（显示快捷键标签）
    if (!isQuickSelectMode.value) {
      isQuickSelectMode.value = true
    }
  }

  // 继续处理其他键盘导航
  searchAndNavManager.handleKeyNavigation(event)
}

/**
 * 处理键盘释放事件
 */
const handleKeyboardUpEvent = (event: KeyboardEvent) => {
  // 检查是否释放了 Ctrl 或 Cmd 键
  if (!event.ctrlKey && !event.metaKey && isQuickSelectMode.value) {
    // 退出快捷选择模式
    isQuickSelectMode.value = false
  }
}

// ==================== 样式初始化 ====================



// ==================== 生命周期钩子 ====================

/**
 * 监听 selectedIndex 变化，自动滚动到可视区域
 */
watch(selectedIndex, () => {
  if (searchAndNavManager.isKeyboardNavigation.value) {
    nextTick(() => {
      scrollAndTagsManager.scrollToSelectedItem()
      searchAndNavManager.setKeyboardNavigation(false)
    })
  }
})

/**
 * 监听 searchText 变化，自动触发搜索
 */
watch(searchAndNavManager.searchText, (newValue) => {
  const shouldTriggerSearch = animationManager.isSearchVisible.value &&
    !animationManager.isOpeningSearch.value &&
    newValue !== undefined

  if (shouldTriggerSearch) {
    searchAndNavManager.debouncedSearch(newValue)
  }
})

/**
 * 组件挂载时的初始化逻辑
 */
onMounted(async () => {
  try {
    // 监听来自主进程的动画触发信号
    const removeSlideInListener = onSimple('trigger-slide-in-animation', () => {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          triggerSlideInAnimation().catch(console.error)
        })
      })
    })

    // 绑定键盘事件监听器
    eventManager.addEventListener(document, 'keydown', (event: Event) => {
      handleKeyboardEvent(event as KeyboardEvent)
    })

    // 绑定键盘释放事件监听器
    eventManager.addEventListener(document, 'keyup', (event: Event) => {
      handleKeyboardUpEvent(event as KeyboardEvent)
    })

    // 绑定点击外部关闭下拉菜单的事件监听器
    eventManager.addEventListener(document, 'click', (event: Event) => {
      const target = event.target as HTMLElement
      const dropdownContainer = target.closest('.dropdown-container')

      if (!dropdownContainer && isDropdownVisible.value) {
        hideDropdownMenu()
      }
    })

    // 添加滚轮事件监听器（支持触摸板手势）
    wheelHandler = (event: WheelEvent) => scrollAndTagsManager.handleWheel(event)
    document.addEventListener('wheel', wheelHandler, { passive: false })

    // 初始化搜索框和标签组的状态
    animationManager.initializeStyles()

    // 并行预加载数据
    const preloadPromises = [
      loadClipboardHistory({
        tagIds: [],
        searchText: ''
      }).catch(() => []),
      scrollAndTagsManager.loadAllTags().catch(() => [])
    ]

    await Promise.allSettled(preloadPromises)

    // 初始化 smooth-scrollbar
    nextTick(() => scrollAndTagsManager.initializeScrollbar())

    // 监听剪贴板历史更新通知
    const removeClipboardUpdateListener = onSimple('clipboard-history-updated', throttledLoadClipboardHistory)

    // 监听标签数据更新通知
    const removeTagsUpdateListener = onSimple('tags-updated', () => {
      scrollAndTagsManager.reloadTags().catch(console.error)
    })

    // 将清理函数添加到事件管理器
    eventManager.addEventListener(window, 'beforeunload', () => {
      removeSlideInListener()
      removeCloseListener?.()
      removeClipboardUpdateListener()
      removeTagsUpdateListener()
    })

  } catch (error) {
    console.error('[前端] Overlay 组件初始化失败:', error)
    throw new OverlayError(
      'Overlay 组件初始化失败',
      OverlayErrorCode.INVALID_STATE,
      { originalError: error }
    )
  }
})

/**
 * 组件卸载时的清理逻辑
 */
onUnmounted(() => {
  try {
    throttledLoadClipboardHistory.cancel()
    searchAndNavManager.cleanup()
    scrollAndTagsManager.destroyScrollbar()

    if (wheelHandler) {
      document.removeEventListener('wheel', wheelHandler)
      wheelHandler = null
    }

  } catch (error) {
    console.error('[前端] Overlay 组件资源清理失败:', error)
  }
})
</script>

<template>
  <div class="overlay-container">
    <div ref="overlayContentRef" class="overlay-content">
      <!-- 顶部工具栏 -->
      <div class="top-toolbar">
        <!-- 搜索图标 -->
        <button class="toolbar-icon search-icon" @click="() => toggleSearch(false)">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8"></circle>
            <path d="m21 21-4.35-4.35"></path>
          </svg>
        </button>

        <!-- 搜索输入框 -->
        <div ref="searchContainerRef" v-show="animationManager.showSearchContainer.value" class="search-container">
          <input type="text" class="search-input" placeholder="Search clipboard history..."
            :value="searchAndNavManager.searchText.value" @input="searchAndNavManager.handleSearchInput"
            @keydown.escape="() => toggleSearch(false)" />
        </div>

        <!-- 标签组 -->
        <div ref="tabsContainerRef" v-show="animationManager.showTabsContainer.value" class="tabs-container">
          <div v-for="tab in scrollAndTagsManager.tabs.value" :key="tab.id" class="tab" :class="{
            active: tab.active,
            'has-count': tab.count > 0,
            'switching': scrollAndTagsManager.isTagSwitching.value && tab.active
          }" tabindex="0" role="button"
            :aria-label="`Switch to ${tab.label} tag${tab.count > 0 ? ` (${tab.count} items)` : ''}`"
            @click="() => scrollAndTagsManager.switchTag(tab.id)" @keydown.enter="() => scrollAndTagsManager.switchTag(tab.id)"
            @keydown.space.prevent="() => scrollAndTagsManager.switchTag(tab.id)">
            <span class="tab-dot" :style="{ backgroundColor: tab.color }"></span>
            <span class="tab-label">{{ tab.label }}</span>
            <span v-if="tab.count > 0" class="tab-count">{{ tab.count }}</span>
            <!-- 加载指示器 -->
            <div v-if="scrollAndTagsManager.isTagSwitching.value && tab.active" class="tab-loading">
              <div class="loading-spinner"></div>
            </div>
          </div>
        </div>

        <!-- 右侧按钮组 -->
        <div class="toolbar-buttons">
          <div class="dropdown-container">
            <button class="toolbar-icon menu-icon" @click="toggleDropdownMenu">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="1"></circle>
                <circle cx="19" cy="12" r="1"></circle>
                <circle cx="5" cy="12" r="1"></circle>
              </svg>
            </button>

            <!-- 下拉菜单 -->
            <div v-if="isDropdownVisible" ref="dropdownMenuRef" class="dropdown-menu">
              <div class="dropdown-item" @click="openSettings">
                <span class="dropdown-icon">
                  <IconSettings />
                </span>
                <span class="dropdown-label">设置</span>
              </div>
              <div class="dropdown-item" @click="clearClipboardHistory">
                <span class="dropdown-icon">
                  <IconDelete />
                </span>
                <span class="dropdown-label">清空历史</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="overlay-body">
        <!-- 空状态 -->
        <div v-if="clipboardHistory.length === 0" class="empty-container">
          <div class="empty-icon">📋</div>
          <p class="empty-text">No clipboard history</p>
          <p class="empty-hint">Copy something to get started</p>
        </div>

        <!-- 剪贴板历史横向滚动 -->
        <div v-else ref="clipboardScrollRef" class="clipboard-scroll-container">
          <transition-group name="clipboard-list" tag="div" class="clipboard-list-wrapper">
            <ClipboardCard v-for="(item, index) in clipboardHistory" :key="item.id" :item="item" :index="index"
              :is-selected="index === selectedIndex" :is-quick-select-mode="isQuickSelectMode"
              :all-tags="scrollAndTagsManager.allTags.value"
              :active-tag-id="scrollAndTagsManager.activeTagId.value"
              :apps="clipboardApps"
              @click="() => restoreClipboardContent(item.id)"
              @restore-as-text="() => restoreClipboardContentAsText(item.id)"
              @delete="() => deleteClipboardItem(item.id)" />
          </transition-group>
        </div>
      </div>
    </div>
  </div>

  <!-- 全局右键菜单 -->
  <ContextMenu />
</template>

<style scoped>
/* 基础容器样式 */
.overlay-container {
  position: fixed;
  top: 0;
  left: 0;
  width: 100vw;
  height: 100vh;
  display: flex;
  justify-content: center;
  align-items: flex-end;
  z-index: 9999;
  background: transparent;
}

.overlay-content {
  width: 100%;
  height: 100vh;
  background: #f9f9f9;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  position: relative;
  transform: translateY(100%);
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
}

/* 顶部工具栏样式 */
.top-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 24px;
  background: #fef9e6;
  border-bottom: 1px solid #f0f0f0;
  backdrop-filter: blur(20px);
  height: 56px;
  min-height: 56px;
  max-height: 56px;
  box-sizing: border-box;
  z-index: 1000;
}

.toolbar-icon {
  background: none;
  border: none;
  cursor: pointer;
  color: #6b7280;
  padding: 8px;
  border-radius: 8px;
  transition: all 0.2s ease;
  display: flex;
  align-items: center;
  justify-content: center;
}

.toolbar-icon:hover {
  background: rgba(0, 0, 0, 0.05);
  color: #374151;
}

.toolbar-buttons {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

/* 下拉菜单容器 */
.dropdown-container {
  position: relative;
}

/* 下拉菜单样式 */
.dropdown-menu {
  position: absolute;
  top: 100%;
  right: 0;
  margin-top: 4px;
  background: rgba(255, 255, 255, 0.95);
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  box-shadow:
    0 4px 20px rgba(0, 0, 0, 0.15),
    0 2px 8px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(10px);
  min-width: 140px;
  padding: 4px 0;
  font-size: 14px;
  user-select: none;
  animation: dropdownFadeIn 0.15s ease-out;
}

@keyframes dropdownFadeIn {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* 下拉菜单项 */
.dropdown-item {
  display: flex;
  align-items: center;
  padding: 8px 12px;
  cursor: pointer;
  transition: background-color 0.2s ease;
  color: #333;
}

.dropdown-item:hover {
  background-color: rgba(0, 123, 255, 0.1);
  color: #007bff;
}

.dropdown-icon {
  margin-right: 8px;
  font-size: 16px;
  width: 16px;
  text-align: center;
}

.dropdown-label {
  flex: 1;
  white-space: nowrap;
}

/* 搜索容器样式 */
.search-container {
  overflow: hidden;
  white-space: nowrap;
  transform-origin: left center;
  width: 0px;
  opacity: 0;
  transform: scaleX(0);
  padding-left: 0px;
  padding-right: 0px;
}

.search-input {
  width: 100%;
  padding: 6px 6px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  font-size: 17px;
  background: white;
  color: #1f2937;
  outline: none;
  transition: all 0.2s ease;
}

.search-input:focus {
  border-color: #3b82f6;
  box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
}

.search-input::placeholder {
  color: #9ca3af;
}

/* 标签容器样式 */
.tabs-container {
  display: flex;
  gap: 8px;
  flex: 1;
  overflow-x: auto;
  padding: 4px 8px;
  transform-origin: left center;
  opacity: 1;
  transform: scaleX(1);
  max-width: 100%;
  align-items: center;
}

.tabs-container::-webkit-scrollbar {
  height: 0;
}

/* 标签样式 */
.tab {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  white-space: nowrap;
  padding: 8px 12px;
  border-radius: 8px;
  position: relative;
  background: transparent;
  border: 1px solid transparent;
  min-width: fit-content;
  width: auto;
}

.tab:hover:not(.switching) {
  background: rgba(255, 255, 255, 0.8);
  border-color: rgba(0, 0, 0, 0.08);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.06);
}

.tab:focus {
  outline: none;
  background: rgba(255, 255, 255, 0.9);
  border-color: #3b82f6;
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.08);
}

.tab.active {
  background: rgba(255, 255, 255, 0.95);
  border-color: rgba(0, 0, 0, 0.12);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.08);
}

.tab.switching {
  pointer-events: none;
  opacity: 0.7;
}

.tab-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tab.active .tab-dot {
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.7);
}

.tab-label {
  font-size: 14px;
  font-weight: 600;
  user-select: none;
  color: #6b7280;
}

.tab.active .tab-label {
  color: #1f2937;
}

.tab-count {
  font-size: 11px;
  font-weight: 600;
  padding: 2px 6px;
  border-radius: 10px;
  min-width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  user-select: none;
  flex-shrink: 0;
  box-sizing: border-box;
  background: rgba(107, 114, 128, 0.2);
  color: #6b7280;
}

.tab.active .tab-count {
  background: rgba(0, 0, 0, 0.1);
  color: #1f2937;
}

/* 加载指示器 */
.tab-loading {
  position: absolute;
  top: 50%;
  right: 8px;
  transform: translateY(-50%);
  width: 12px;
  height: 12px;
  flex-shrink: 0;
}

.loading-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(0, 0, 0, 0.1);
  border-left-color: #3b82f6;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

:global(.clipboard-scroll-container .scroll-content) {
  transition: transform 0.4s ease;

}

/* 主体区域样式 */
.overlay-body {
  flex: 1;
  overflow-y: hidden;
  background: #f9f9f9;
}

/* 空状态样式 */
.empty-container {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 70vh;
  gap: 16px;
}

.empty-icon {
  font-size: 48px;
  opacity: 0.6;
}

.empty-text {
  font-size: 20px;
  font-weight: 600;
  color: #1f2937;
  margin: 0;
}

.empty-hint {
  font-size: 14px;
  color: #6b7280;
  margin: 0;
}

/* 剪贴板滚动容器样式 */
.clipboard-scroll-container {
  padding-top: 15px;
  padding-left: 20px;
  padding-bottom: 15px;
  overflow: hidden;
}

.clipboard-list-wrapper {
  display: flex;
  gap: 16px;
}

.clipboard-list-wrapper::after {
  content: '';
  width: 4px;
  flex-shrink: 0;
}

/* 剪贴板列表过渡动画 */
.clipboard-list-enter-active,
.clipboard-list-leave-active {
  transition: all 0.3s ease;
}

.clipboard-list-enter-from {
  opacity: 0;
  transform: translateY(20px);
}

.clipboard-list-leave-to {
  opacity: 0;
  transform: translateY(-20px);
}

.clipboard-list-move {
  transition: transform 0.3s ease;
}

/* 隐藏 smooth-scrollbar 滚动条 */
.scrollbar-track,
.scrollbar-thumb,
.scrollbar-track-x,
.scrollbar-track-y,
.scrollbar-thumb-x,
.scrollbar-thumb-y {
  display: none !important;
  opacity: 0 !important;
  visibility: hidden !important;
}

/* 针对容器内的 smooth-scrollbar 元素 */
.clipboard-scroll-container :deep(.scrollbar-track),
.clipboard-scroll-container :deep(.scrollbar-thumb),
.clipboard-scroll-container :deep(.scrollbar-track-x),
.clipboard-scroll-container :deep(.scrollbar-track-y),
.clipboard-scroll-container :deep(.scrollbar-thumb-x),
.clipboard-scroll-container :deep(.scrollbar-thumb-y) {
  display: none !important;
  opacity: 0 !important;
  visibility: hidden !important;
}

/* 响应式设计 */
@media (max-width: 768px) {
  .tabs-container {
    gap: 4px;
    padding: 2px 4px;
  }

  .tab {
    padding: 6px 8px;
    gap: 6px;
  }

  .tab-label {
    font-size: 13px;
  }

  .tab-count {
    font-size: 10px;
    min-width: 16px;
    height: 16px;
    padding: 1px 4px;
  }

  .tab-dot {
    width: 6px;
    height: 6px;
  }
}

/* 减少动画偏好支持 */
@media (prefers-reduced-motion: reduce) {
  .tab,
  .tab-dot,
  .tab-label,
  .tab-count,
  .toolbar-icon,
  .search-input {
    transition: none;
    animation: none;
  }
}
</style>
