<script setup lang="ts">
import { ref, nextTick, watch, onMounted, onUnmounted } from 'vue'
import { send, onSimple } from '@renderer/api'
import { clipboard } from '@renderer/api/renderer-api'
import type { ClipboardItem, ClipboardHistoryQueryOptions } from '@renderer/api/types'
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

// ==================== DOM 引用 ====================
const overlayContentRef = ref<HTMLElement>()
const clipboardScrollRef = ref<HTMLElement>()
const searchContainerRef = ref<HTMLElement>()
const tabsContainerRef = ref<HTMLElement>()

// ==================== 核心状态 ====================
const clipboardHistory = ref<ClipboardItem[]>([])
const selectedIndex = ref(0)

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
    const history = await clipboard.searchHistory(queryOptions)

    // 类型验证
    if (!Array.isArray(history)) {
      throw new OverlayError(
        '剪贴板历史数据格式无效：不是数组',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { receivedType: typeof history, queryOptions }
      )
    }

    // 验证每个项目的类型
    const validHistory = history.filter((item, index) => {
      if (!isClipboardItem(item)) {
        console.warn(`[前端] 跳过无效的剪贴板项目 (索引 ${index}):`, item)
        return false
      }
      return true
    })

    // 更新数据
    clipboardHistory.value = validHistory

    // 重置选中索引
    if (selectedIndex.value >= validHistory.length) {
      selectedIndex.value = Math.max(0, validHistory.length - 1)
    }

    // 确保选中的项目可见
    nextTick(() => {
      scrollAndTagsManager.initializeScrollbar()
      scrollAndTagsManager.scrollToSelectedItem()
    })

    console.log(`[前端] 成功加载 ${validHistory.length} 条剪贴板历史记录`)
  } catch (error) {
    console.error('获取剪贴板历史失败:', error)

    const overlayError = error instanceof OverlayError
      ? error
      : new OverlayError(
        '加载剪贴板历史时发生未知错误',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { originalError: error, options }
      )

    // 重置为空数组
    clipboardHistory.value = []
    selectedIndex.value = 0

    console.error('[前端] 剪贴板历史加载错误:', overlayError)
  }
}

// 创建节流的剪贴板历史更新函数
const throttledLoadClipboardHistory = useThrottle(
  () => {
    console.log('[前端] 执行节流的剪贴板历史更新')
    loadClipboardHistory().catch(error => {
      console.warn('[前端] 节流重新加载剪贴板历史失败:', error)
    })
  },
  500
)

// ==================== 初始化滚动和标签管理 ====================
const scrollAndTagsManager = useScrollAndTags(
  clipboardScrollRef,
  clipboardHistory,
  selectedIndex,
  animationManager.currentAnimationState,
  loadClipboardHistory
)

// ==================== 事件监听器管理 ====================

/**
 * 设置关闭事件监听器
 * 防止重复设置监听器
 */
const setupCloseEventListeners = () => {
  // 如果已经设置过监听器，跳过重复设置
  if (isCloseListenerSetup) {
    console.log('[前端] 关闭事件监听器已设置，跳过重复设置')
    return
  }

  // 如果存在旧的监听器，先清理
  if (removeCloseListener) {
    console.log('[前端] 清理已存在的关闭事件监听器')
    removeCloseListener()
    removeCloseListener = null
  }

  // 监听来自主进程的关闭窗口信号
  console.log('[前端] 设置关闭事件监听器')
  removeCloseListener = onSimple('trigger-close-window', handleCloseWindow)
  isCloseListenerSetup = true
}

// ==================== 核心功能函数 ====================

/**
 * 还原剪贴板内容
 */
const restoreClipboardContent = async (itemId: string): Promise<void> => {
  try {
    if (!itemId || typeof itemId !== 'string') {
      throw new OverlayError(
        '无效的剪贴板项目ID',
        OverlayErrorCode.INVALID_STATE,
        { itemId, type: typeof itemId }
      )
    }

    console.log(`[前端] 开始还原剪贴板内容，ID: ${itemId}`)
    const success = await clipboard.restore(itemId)

    if (success) {
      console.log('[前端] 剪贴板内容已成功还原')
      closeWindow()
    } else {
      throw new OverlayError(
        '剪贴板还原操作失败',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { itemId }
      )
    }
  } catch (error) {
    console.error('[前端] 还原剪贴板失败:', error)
    const overlayError = error instanceof OverlayError
      ? error
      : new OverlayError(
        '还原剪贴板时发生未知错误',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { originalError: error, itemId }
      )
    console.error('[前端] 剪贴板还原错误:', overlayError)
  }
}

/**
 * 还原剪贴板内容为纯文本
 */
const restoreClipboardContentAsText = async (itemId: string): Promise<void> => {
  try {
    if (!itemId || typeof itemId !== 'string') {
      throw new OverlayError(
        '无效的剪贴板项目ID',
        OverlayErrorCode.INVALID_STATE,
        { itemId, type: typeof itemId }
      )
    }

    console.log(`[前端] 开始还原剪贴板内容为纯文本，ID: ${itemId}`)
    const success = await clipboard.restoreAsText(itemId)

    if (success) {
      console.log('[前端] 剪贴板内容已成功还原为纯文本')
      closeWindow()
    } else {
      throw new OverlayError(
        '剪贴板还原为纯文本操作失败',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { itemId }
      )
    }
  } catch (error) {
    console.error('[前端] 还原剪贴板为纯文本失败:', error)
    const overlayError = error instanceof OverlayError
      ? error
      : new OverlayError(
        '还原剪贴板为纯文本时发生未知错误',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { originalError: error, itemId }
      )
    console.error('[前端] 剪贴板还原为纯文本错误:', overlayError)
  }
}

/**
 * 关闭窗口
 */
const closeWindow = async () => {
  if (!animationManager.canStartClosing()) {
    console.log(`[前端] 当前状态为 ${animationManager.currentAnimationState.value}，无法开始关闭`)
    return
  }

  console.log('[前端] 开始关闭窗口流程')
  animationManager.setAnimationState(AnimationState.SLIDING_OUT)

  const { play: playSlideOut, playState } = animationManager.createSlideOutAnimation()
  playSlideOut()

  // 等待动画完成
  watch(playState, (newState) => {
    if (newState === 'finished') {
      animationManager.resetOverlayState()
      send('close-overlay-window')
    }
  })
}

/**
 * 切换搜索框显示状态
 */
const toggleSearch = async (withBuffer = false): Promise<void> => {
  try {
    if (animationManager.isSearchAnimating.value) {
      console.log('[前端] 搜索动画进行中，忽略切换请求')
      return
    }

    const isCurrentlyVisible = animationManager.isSearchVisible.value
    console.log(`[前端] 切换搜索框状态: ${isCurrentlyVisible ? '隐藏' : '显示'}`)

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
    new Promise<void>((resolve) => {
      watch(searchCollapseState, (newState) => {
        if (newState === 'finished') resolve()
      })
    }),
    new Promise<void>((resolve) => {
      watch(tabsShowState, (newState) => {
        if (newState === 'finished') resolve()
      })
    })
  ])

  animationManager.setSearchVisible(false)
  animationManager.setSearchAnimating(false)
  console.log('[前端] 搜索框隐藏动画完成')
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
    new Promise<void>((resolve) => {
      watch(tabsHideState, (newState) => {
        if (newState === 'finished') resolve()
      })
    }),
    new Promise<void>((resolve) => {
      watch(searchExpandState, (newState) => {
        if (newState === 'finished') resolve()
      })
    })
  ])

  animationManager.setSearchAnimating(false)
  animationManager.setOpeningSearch(false)

  if (withBuffer) {
    searchAndNavManager.applyInputBuffer()
  }

  searchAndNavManager.focusSearchInput()
  console.log('[前端] 搜索框显示动画完成')
}

// ==================== 搜索回调函数 ====================
const performSearchCallback = async (text: string): Promise<void> => {
  try {
    if (text.trim()) {
      console.log(`[前端] 执行搜索，关键词: "${text.trim()}"`)
      await loadClipboardHistory({
        searchText: text.trim(),
        limit: 50,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })
    } else {
      console.log('[前端] 搜索文本为空，显示默认历史记录')
      await loadClipboardHistory()
    }
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

// ==================== 动画流程函数 ====================

/**
 * 触发滑入动画
 */
const triggerSlideInAnimation = async (): Promise<void> => {
  try {
    console.log('[前端] 收到触发滑入动画信号')

    if (animationManager.isAnimating()) {
      console.log(`[前端] 正在进行动画，忽略滑入请求，当前状态: ${animationManager.currentAnimationState.value}`)
      return
    }

    console.log('[前端] 开始滑入动画流程')

    // 重置搜索状态
    if (!searchAndNavManager.searchText.value.trim()) {
      console.log('[前端] 搜索框无文字，重置为关闭状态')
      animationManager.resetSearchStyles()
    }

    // 清空输入缓冲区和相关状态
    searchAndNavManager.resetSearchState()

    // 设置状态为正在滑入
    animationManager.setAnimationState(AnimationState.SLIDING_IN)

    // 开始滑入动画
    const { play: playSlideIn, playState } = animationManager.createSlideInAnimation()
    playSlideIn()

    // 等待滑入动画完成（带超时）
    await withTimeout(
      new Promise<void>((resolve, reject) => {
        watch(playState, (newState) => {
          if (newState === 'finished') {
            console.log('[前端] 滑入动画完成')
            resolve()
          } else if (newState === 'idle') {
            reject(new Error('滑入动画意外停止'))
          }
        })
      }),
      5000,
      '滑入动画超时'
    )

    console.log('[前端] 设置为活跃状态')
    animationManager.setAnimationState(AnimationState.ACTIVE)

    // 载入动画完成后才设置关闭事件监听器
    setupCloseEventListeners()

    // 通知后端动画完成
    send('open-overlay-finished')
    console.log('[前端] 滑入动画流程完成')
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
    console.log(`[前端] 收到关闭窗口信号，当前状态: ${animationManager.currentAnimationState.value}，强制关闭: ${force}`)

    if (animationManager.currentAnimationState.value !== AnimationState.ACTIVE) {
      console.log(`[前端] 当前状态为 ${animationManager.currentAnimationState.value}，忽略关闭请求`)
      return
    }

    if (!force) {
      // 检查搜索框状态
      if (animationManager.isSearchVisible.value && searchAndNavManager.searchText.value.trim()) {
        console.log('[前端] 搜索框有值，清空搜索框并返回')
        searchAndNavManager.resetSearchState()
        await performSearchCallback('')
        return
      }

      if (animationManager.isSearchVisible.value && !searchAndNavManager.searchText.value.trim()) {
        console.log('[前端] 搜索框无值，切换为隐藏状态')
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
      console.error('[前端] 强制关闭窗口也失败:', closeError)
      send('close-overlay-window')
    }
  }
}

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
  if (animationManager.isSearchVisible.value &&
      !animationManager.isOpeningSearch.value &&
      newValue !== undefined) {
    searchAndNavManager.debouncedSearch(newValue)
  }
})

/**
 * 组件挂载时的初始化逻辑
 */
onMounted(async () => {
  try {
    console.log('[前端] 开始初始化 Overlay 组件')

    // 监听来自主进程的动画触发信号
    const removeSlideInListener = onSimple('trigger-slide-in-animation', () => {
      // 使用双重 RAF 确保动画在下一帧执行
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          triggerSlideInAnimation().catch(error => {
            console.error('[前端] 滑入动画触发失败:', error)
          })
        })
      })
    })

    // 注意：关闭事件监听器将在载入动画完成后通过 setupCloseEventListeners() 设置

    // 立即绑定键盘事件监听器（在组件挂载时就绑定）
    eventManager.addEventListener(document, 'keydown', (event: Event) => {
      searchAndNavManager.handleKeyNavigation(event as KeyboardEvent)
    })
    console.log('[前端] 键盘事件监听器已绑定')

    // 立即添加滚轮事件监听器（支持触摸板手势）
    // 使用直接绑定以确保事件不会丢失
    wheelHandler = (event: WheelEvent) => {
      console.log('[前端] 滚轮事件触发:', {
        deltaX: event.deltaX,
        deltaY: event.deltaY,
        target: event.target?.constructor.name
      })
      scrollAndTagsManager.handleWheel(event)
    }
    document.addEventListener('wheel', wheelHandler, { passive: false })
    console.log('[前端] 滚轮事件监听器已绑定到 document')

    // 初始化搜索框和标签组的状态
    animationManager.initializeStyles()
    console.log('[前端] 搜索动画样式已初始化')

    // 并行预加载数据
    const preloadPromises = [
      loadClipboardHistory().catch(error => {
        console.warn('[前端] 预加载剪贴板历史失败:', error)
        return []
      }),
      scrollAndTagsManager.loadAllTags().catch(error => {
        console.warn('[前端] 预加载标签数据失败:', error)
        return []
      })
    ]

    await Promise.allSettled(preloadPromises)
    console.log('[前端] 数据预加载完成')

    // 初始化 smooth-scrollbar
    nextTick(() => {
      scrollAndTagsManager.initializeScrollbar()
      console.log('[前端] smooth-scrollbar 已初始化')
    })

    // 监听剪贴板历史更新通知
    const removeClipboardUpdateListener = onSimple('clipboard-history-updated', (data) => {
      console.log('[前端] 收到剪贴板历史更新通知:', data)
      throttledLoadClipboardHistory()
    })

    // 监听标签数据更新通知
    const removeTagsUpdateListener = onSimple('tags-updated', (data) => {
      console.log('[前端] 收到标签数据更新通知:', data)
      scrollAndTagsManager.reloadTags().catch(error => {
        console.warn('[前端] 重新加载标签数据失败:', error)
      })
    })

    // 将清理函数添加到事件管理器
    eventManager.addEventListener(window, 'beforeunload', () => {
      removeSlideInListener()
      removeCloseListener?.()
      removeClipboardUpdateListener()
      removeTagsUpdateListener()
    })

    console.log('[前端] Overlay 组件初始化完成')
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
    console.log('[前端] 开始清理 Overlay 组件资源')

    // 清理节流函数
    throttledLoadClipboardHistory.cancel()
    console.log('[前端] 节流函数已清理')

    // 清理搜索相关资源
    searchAndNavManager.cleanup()
    console.log('[前端] 搜索逻辑资源已清理')

    // 清理 smooth-scrollbar 实例
    scrollAndTagsManager.destroyScrollbar()
    console.log('[前端] smooth-scrollbar 实例已清理')

    // 清理滚轮事件监听器
    if (wheelHandler) {
      document.removeEventListener('wheel', wheelHandler)
      wheelHandler = null
      console.log('[前端] 滚轮事件监听器已清理')
    }

    // 事件管理器会自动清理所有事件监听器
    console.log('[前端] Overlay 组件资源清理完成')
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
          <button class="toolbar-icon add-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19"></line>
              <line x1="5" y1="12" x2="19" y2="12"></line>
            </svg>
          </button>
          <button class="toolbar-icon menu-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="12" cy="12" r="1"></circle>
              <circle cx="19" cy="12" r="1"></circle>
              <circle cx="5" cy="12" r="1"></circle>
            </svg>
          </button>
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
              :is-selected="index === selectedIndex" @click="() => restoreClipboardContent(item.id)"
              @restore-as-text="() => restoreClipboardContentAsText(item.id)" />
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
.clipboard-scroll-container :deep(.scrollbar-track),
.clipboard-scroll-container :deep(.scrollbar-thumb),
.clipboard-scroll-container :deep(.scrollbar-track-x),
.clipboard-scroll-container :deep(.scrollbar-track-y),
.clipboard-scroll-container :deep(.scrollbar-thumb-x),
.clipboard-scroll-container :deep(.scrollbar-thumb-y) {
  display: none !important;
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
