<script setup lang="ts">
import { nextTick } from 'vue'
import { send, onSimple } from '@renderer/api'
import { clipboard } from '@renderer/api/renderer-api'
import type { ClipboardItem, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import {
  OverlayError,
  OverlayErrorCode,
  isClipboardItem
} from '@renderer/types/overlay'

// 导入 composables
import { useOverlayAnimation, AnimationState } from '@renderer/composables/useOverlayAnimation'
import { useSearchAnimation } from '@renderer/composables/useSearchAnimation'
import { useSearch } from '@renderer/composables/useSearch'
import { useKeyboardNavigation } from '@renderer/composables/useKeyboardNavigation'
import { useScrollManagement } from '@renderer/composables/useScrollManagement'
import { useTagManagement } from '@renderer/composables/useTagManagement'
import { useThrottle } from '@renderer/composables/usePerformanceOptimization'
// import { useEventListenerManager, useMemoryLeakGuard } from '@renderer/composables/usePerformanceOptimization'

// DOM 引用
const overlayContentRef = ref<HTMLElement>()
const clipboardScrollRef = ref<HTMLElement>()
const searchContainerRef = ref<HTMLElement>()
const tabsContainerRef = ref<HTMLElement>()

// 剪贴板历史记录
const clipboardHistory = ref<ClipboardItem[]>([])
const selectedIndex = ref(0)

// 存储监听器取消函数
let removeSlideInListener: (() => void) | null = null
let removeCloseListener: (() => void) | null = null
let removeClipboardUpdateListener: (() => void) | null = null
let removeTagsUpdateListener: (() => void) | null = null

// 监听器状态标志
let isCloseListenerSetup = false

// 初始化 composables
const overlayAnimation = useOverlayAnimation(overlayContentRef)
const searchAnimation = useSearchAnimation(searchContainerRef, tabsContainerRef)

// 创建节流的剪贴板历史更新函数（500ms 节流，避免频繁更新）
const [throttledLoadClipboardHistory, cancelThrottledLoad] = useThrottle(
  () => {
    console.log('[前端] 执行节流的剪贴板历史更新')
    loadClipboardHistory().catch(error => {
      console.warn('[前端] 节流重新加载剪贴板历史失败:', error)
    })
  },
  500 // 500ms 节流间隔
)

// 获取剪贴板历史记录（带类型验证和错误处理）
const loadClipboardHistory = async (options?: ClipboardHistoryQueryOptions): Promise<void> => {
  try {
    // 使用 searchHistory 替代 getHistory，支持更多筛选选项
    const defaultOptions: ClipboardHistoryQueryOptions = {
      limit: 200, // 获取最近200条记录
      orderBy: 'last_used',
      orderDirection: 'DESC'
    }

    const queryOptions = { ...defaultOptions, ...options }
    const history = await clipboard.searchHistory(queryOptions)

    // 类型验证：确保返回的数据是有效的剪贴板项目数组
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

    // 直接更新数据
    clipboardHistory.value = validHistory

    // 重置选中索引，但保持在有效范围内
    if (selectedIndex.value >= validHistory.length) {
      selectedIndex.value = Math.max(0, validHistory.length - 1)
    }

    // 数据加载完成后，确保选中的项目可见
    nextTick(() => {
      // 确保 smooth-scrollbar 已初始化
      scrollManagement.initializeScrollbar()
      scrollManagement.scrollToSelectedItem()
    })

    console.log(`[前端] 成功加载 ${validHistory.length} 条剪贴板历史记录`)
  } catch (error) {
    console.error('获取剪贴板历史失败:', error)

    // 创建结构化错误
    const overlayError = error instanceof OverlayError
      ? error
      : new OverlayError(
        '加载剪贴板历史时发生未知错误',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { originalError: error, options }
      )

    // 重置为空数组，确保UI不会崩溃
    clipboardHistory.value = []
    selectedIndex.value = 0

    // 可以在这里添加用户通知逻辑
    console.error('[前端] 剪贴板历史加载错误:', overlayError)
  }
}

const tagManagement = useTagManagement(loadClipboardHistory)

/**
 * 执行搜索的回调函数
 * 根据搜索文本过滤剪贴板历史记录
 *
 * @param text - 搜索文本，为空时显示所有记录
 */
const performSearchCallback = async (text: string): Promise<void> => {
  try {
    searchLogic._setSearching(true)

    if (text.trim()) {
      // 有搜索文本时进行搜索
      console.log(`[前端] 执行搜索，关键词: "${text.trim()}"`)
      await loadClipboardHistory({
        searchText: text.trim(),
        limit: 50, // 搜索时显示更多结果以提高匹配度
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })
    } else {
      // 搜索文本为空时显示默认历史记录
      console.log('[前端] 搜索文本为空，显示默认历史记录')
      await loadClipboardHistory()
    }
  } catch (error) {
    console.error('[前端] 搜索执行失败:', error)

    // 搜索失败时确保UI状态正确
    searchLogic._setSearching(false)
    throw new OverlayError(
      '搜索操作失败',
      OverlayErrorCode.SEARCH_FAILED,
      { searchText: text, originalError: error }
    )
  } finally {
    searchLogic._setSearching(false)
  }
}

const searchLogic = useSearch(performSearchCallback)
const scrollManagement = useScrollManagement(
  clipboardScrollRef,
  clipboardHistory,
  selectedIndex,
  overlayAnimation.currentAnimationState
)

// 关闭窗口
const closeWindow = async () => {
  // 检查是否可以开始关闭
  if (!overlayAnimation.canStartClosing()) {
    console.log(`[前端] 当前状态为 ${overlayAnimation.currentAnimationState.value}，无法开始关闭`)
    return
  }

  console.log('[前端] 开始关闭窗口流程')

  // 设置状态为正在滑出
  overlayAnimation.setAnimationState(AnimationState.SLIDING_OUT)

  const { play: playSlideOut, playState } = overlayAnimation.createSlideOutAnimation()
  playSlideOut()

  // 等待动画完成后关闭窗口
  watch(playState, (newState) => {
    if (newState === 'finished') {
      // 清理关闭事件监听器
      if (removeCloseListener) {
        removeCloseListener()
        removeCloseListener = null
        isCloseListenerSetup = false
        console.log('[前端] 关闭窗口时清理关闭事件监听器')
      }

      overlayAnimation.resetState()
      send('close-overlay-window')
    }
  })
}

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



// 还原剪贴板内容（带类型验证和错误处理）
const restoreClipboardContent = async (itemId: string): Promise<void> => {
  try {
    // 参数验证
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
      // 还原成功后关闭窗口
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

    // 创建结构化错误
    const overlayError = error instanceof OverlayError
      ? error
      : new OverlayError(
        '还原剪贴板时发生未知错误',
        OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
        { originalError: error, itemId }
      )

    console.error('[前端] 剪贴板还原错误:', overlayError)
    // 可以在这里添加用户通知逻辑，比如显示错误提示
  }
}

/**
 * 切换搜索框显示状态
 * 在搜索框和标签组之间进行平滑切换
 *
 * @param withBuffer - 是否应用输入缓冲区的内容到搜索框
 */
const toggleSearch = async (withBuffer = false): Promise<void> => {
  try {
    // 防止重复触发动画
    if (searchAnimation.isSearchAnimating.value) {
      console.log('[前端] 搜索动画进行中，忽略切换请求')
      return
    }

    const isCurrentlyVisible = searchAnimation.isSearchVisible.value
    console.log(`[前端] 切换搜索框状态: ${isCurrentlyVisible ? '隐藏' : '显示'}`)

    if (isCurrentlyVisible) {
      // 隐藏搜索框，显示标签组
      await hideSearchBox()
    } else {
      // 显示搜索框，隐藏标签组
      await showSearchBox(withBuffer)
    }
  } catch (error) {
    console.error('[前端] 搜索框切换失败:', error)

    // 重置动画状态，防止UI卡住
    searchAnimation._setSearchAnimating(false)
    searchAnimation._setOpeningSearch(false)

    throw new OverlayError(
      '搜索框切换动画失败',
      OverlayErrorCode.ANIMATION_FAILED,
      { withBuffer, originalError: error }
    )
  }
}

/**
 * 隐藏搜索框并显示标签组
 */
const hideSearchBox = async (): Promise<void> => {
  searchAnimation._setSearchAnimating(true)

  // 同时播放搜索框缩回动画和标签组显示动画
  const { play: playSearchCollapse, playState: searchCollapseState } = searchAnimation.createSearchCollapseAnimation()
  const { play: playTabsShow, playState: tabsShowState } = searchAnimation.createTabsShowAnimation()

  playSearchCollapse()
  playTabsShow()

  // 等待两个动画都完成
  await Promise.all([
    new Promise<void>((resolve) => {
      watch(searchCollapseState, (newState) => {
        if (newState === 'finished') {
          resolve()
        }
      })
    }),
    new Promise<void>((resolve) => {
      watch(tabsShowState, (newState) => {
        if (newState === 'finished') {
          resolve()
        }
      })
    })
  ])

  // 动画完成后设置状态
  searchAnimation._setSearchVisible(false)
  searchAnimation._setSearchAnimating(false)
  console.log('[前端] 搜索框隐藏动画完成')
}

/**
 * 显示搜索框并隐藏标签组
 *
 * @param withBuffer - 是否应用输入缓冲区的内容
 */
const showSearchBox = async (withBuffer: boolean): Promise<void> => {
  searchAnimation._setOpeningSearch(true)
  searchAnimation._setSearchAnimating(true)
  searchAnimation._setSearchVisible(true)

  // 等待DOM更新
  await nextTick()

  // 同时播放标签组隐藏动画和搜索框展开动画
  const { play: playTabsHide, playState: tabsHideState } = searchAnimation.createTabsHideAnimation()
  const { play: playSearchExpand, playState: searchExpandState } = searchAnimation.createSearchExpandAnimation()

  playTabsHide()
  playSearchExpand()

  // 等待两个动画都完成
  await Promise.all([
    new Promise<void>((resolve) => {
      watch(tabsHideState, (newState) => {
        if (newState === 'finished') {
          resolve()
        }
      })
    }),
    new Promise<void>((resolve) => {
      watch(searchExpandState, (newState) => {
        if (newState === 'finished') {
          resolve()
        }
      })
    })
  ])

  // 动画完成后设置状态
  searchAnimation._setSearchAnimating(false)
  searchAnimation._setOpeningSearch(false)

  // 如果有缓冲的输入，应用到搜索框
  if (withBuffer) {
    searchLogic.applyInputBuffer()
  }

  // 聚焦搜索框
  searchLogic.focusSearchInput()
  console.log('[前端] 搜索框显示动画完成')
}

// 初始化键盘导航
const keyboardNavigation = useKeyboardNavigation(
  clipboardHistory,
  selectedIndex,
  restoreClipboardContent,
  closeWindow,
  () => toggleSearch(),
  (event: KeyboardEvent) => {
    if (searchLogic.isInputCharacter(event)) {
      searchLogic.handleInputCharacter(
        event,
        searchAnimation.isSearchVisible.value,
        searchAnimation.isOpeningSearch.value,
        searchAnimation.isSearchAnimating.value,
        toggleSearch
      )
    }
  },
  searchAnimation.isSearchVisible
)


/**
 * 包装函数：处理搜索框切换（用于模板）
 * 提供简化的接口给模板使用
 */
const handleToggleSearch = (): void => {
  toggleSearch(false).catch(error => {
    console.error('[前端] 搜索框切换失败:', error)
  })
}

/**
 * 包装函数：处理搜索框ESC键（用于模板）
 * 当用户按下ESC键时关闭搜索框
 */
const handleSearchEscape = (): void => {
  toggleSearch(false).catch(error => {
    console.error('[前端] 搜索框ESC关闭失败:', error)
  })
}

/**
 * 包装函数：处理标签切换（用于模板）
 * 提供错误处理的标签切换功能
 */
const handleTagSwitch = (tagId: string): void => {
  tagManagement.switchTag(tagId).catch(error => {
    console.error(`[前端] 切换到标签 ${tagId} 失败:`, error)
  })
}

/**
 * 包装函数：处理剪贴板项目点击（用于模板）
 * 提供错误处理的剪贴板还原功能
 */
const handleClipboardItemClick = (itemId: string): void => {
  restoreClipboardContent(itemId).catch(error => {
    console.error(`[前端] 还原剪贴板项目 ${itemId} 失败:`, error)
  })
}

/**
 * 触发滑入动画
 * 处理 overlay 窗口的滑入动画流程，包括状态重置和动画执行
 */
const triggerSlideInAnimation = async (): Promise<void> => {
  try {
    console.log('[前端] 收到触发滑入动画信号')

    // 检查当前状态，防止重复触发
    if (overlayAnimation.isAnimating()) {
      console.log(`[前端] 正在进行动画，忽略滑入请求，当前状态: ${overlayAnimation.currentAnimationState.value}`)
      return
    }

    console.log('[前端] 开始滑入动画流程')

    // 重置搜索状态：如果搜索框无文字，默认关闭搜索框
    if (!searchLogic.searchText.value.trim()) {
      console.log('[前端] 搜索框无文字，重置为关闭状态')
      searchAnimation.resetSearchStyles()
    }

    // 清空输入缓冲区和相关状态
    searchLogic._clearInputBuffer()

    // 设置状态为正在滑入
    overlayAnimation.setAnimationState(AnimationState.SLIDING_IN)

    // 开始滑入动画
    const { play: playSlideIn, playState } = overlayAnimation.createSlideInAnimation()
    playSlideIn()

    // 等待滑入动画完成
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('滑入动画超时'))
      }, 5000) // 5秒超时

      watch(playState, (newState) => {
        if (newState === 'finished') {
          clearTimeout(timeout)
          console.log('[前端] 滑入动画完成')
          resolve()
        } else if (newState === 'idle') {
          clearTimeout(timeout)
          reject(new Error('滑入动画意外停止'))
        }
      })
    })

    console.log('[前端] 设置为活跃状态')
    // 设置状态为活跃（可交互）
    overlayAnimation.setAnimationState(AnimationState.ACTIVE)

    // 载入动画完成后才设置关闭事件监听器
    setupCloseEventListeners()

    // 通知后端动画完成
    send('open-overlay-finished')
    console.log('[前端] 滑入动画流程完成')
  } catch (error) {
    console.error('[前端] 滑入动画失败:', error)

    // 重置状态，防止UI卡住
    overlayAnimation.resetState()

    throw new OverlayError(
      '滑入动画执行失败',
      OverlayErrorCode.ANIMATION_FAILED,
      { originalError: error }
    )
  }
}

/**
 * 处理主进程发送的关闭窗口事件
 * 根据当前状态和搜索框状态决定关闭行为
 *
 * @param force - 是否强制关闭，忽略搜索框状态
 */
const handleCloseWindow = async (force: boolean = false): Promise<void> => {
  try {
    console.log(`[前端] 收到关闭窗口信号，当前状态: ${overlayAnimation.currentAnimationState.value}，强制关闭: ${force}`)

    // 只有在活跃状态时才响应关闭事件
    if (overlayAnimation.currentAnimationState.value !== AnimationState.ACTIVE) {
      console.log(`[前端] 当前状态为 ${overlayAnimation.currentAnimationState.value}，忽略关闭请求`)
      return
    }

    if (!force) {
      // 检查搜索框状态
      if (searchAnimation.isSearchVisible.value && searchLogic.searchText.value.trim()) {
        // 搜索框为展示状态且有值，清空搜索框的值并返回
        console.log('[前端] 搜索框有值，清空搜索框并返回')
        searchLogic.resetSearchState()
        // 触发搜索以重新加载默认数据
        await performSearchCallback('')
        return
      }

      // 如果搜索框没有值，需要切换搜索框状态为隐藏状态
      if (searchAnimation.isSearchVisible.value && !searchLogic.searchText.value.trim()) {
        console.log('[前端] 搜索框无值，切换为隐藏状态')
        await toggleSearch()
        return
      }
    }

    // 执行关闭窗口
    await closeWindow()
  } catch (error) {
    console.error('[前端] 处理关闭窗口事件失败:', error)

    // 即使出错也要尝试关闭窗口
    try {
      await closeWindow()
    } catch (closeError) {
      console.error('[前端] 强制关闭窗口也失败:', closeError)
      // 最后的手段：直接通知后端关闭
      send('close-overlay-window')
    }
  }
}

// 监听 selectedIndex 变化，自动滚动到可视区域
watch(selectedIndex, () => {
  // 只有在键盘导航时才自动滚动
  if (keyboardNavigation.isKeyboardNavigation.value) {
    // 延迟执行，确保 DOM 更新完成
    nextTick(() => {
      scrollManagement.scrollToSelectedItem()
      // 重置标志
      keyboardNavigation._setKeyboardNavigation(false)
    })
  }
})

// 监听 searchText 变化，自动触发搜索（用于缓冲区应用时）
watch(searchLogic.searchText, (newValue) => {
  // 只有当搜索框可见且不是正在打开时才触发搜索
  if (searchAnimation.isSearchVisible.value && !searchAnimation.isOpeningSearch.value && newValue !== undefined) {
    searchLogic.debouncedSearch(newValue)
  }
})

/**
 * 组件挂载时的初始化逻辑
 * 设置事件监听器、预加载数据、初始化UI状态
 */
onMounted(async () => {
  try {
    console.log('[前端] 开始初始化 Overlay 组件')

    // 监听来自主进程的动画触发信号
    removeSlideInListener = onSimple('trigger-slide-in-animation', () => {
      // 使用双重 RAF 确保动画在下一帧执行
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          triggerSlideInAnimation().catch(error => {
            console.error('[前端] 滑入动画触发失败:', error)
          })
        })
      })
    })

    // 在组件挂载时就绑定键盘事件监听器
    document.addEventListener('keydown', keyboardNavigation.handleKeyNavigation)
    console.log('[前端] 键盘事件监听器已绑定')

    // 添加滚轮事件监听器（支持触摸板手势）
    document.addEventListener('wheel', scrollManagement.handleWheel, { passive: false })
    console.log('[前端] 滚轮事件监听器已绑定')

    // 初始化搜索框和标签组的状态
    searchAnimation.initializeStyles()
    console.log('[前端] 搜索动画样式已初始化')

    // 并行预加载数据以提高性能
    const preloadPromises = [
      loadClipboardHistory().catch(error => {
        console.warn('[前端] 预加载剪贴板历史失败:', error)
        return [] // 返回空数组作为默认值
      }),
      tagManagement.loadAllTags().catch(error => {
        console.warn('[前端] 预加载标签数据失败:', error)
        return [] // 返回空数组作为默认值
      })
    ]

    await Promise.allSettled(preloadPromises)
    console.log('[前端] 数据预加载完成')

    // 初始化 smooth-scrollbar（在数据加载完成后）
    nextTick(() => {
      scrollManagement.initializeScrollbar()
      console.log('[前端] smooth-scrollbar 已初始化')
    })

    // 监听剪贴板历史更新通知（使用节流避免频繁更新）
    removeClipboardUpdateListener = onSimple('clipboard-history-updated', (data) => {
      console.log('[前端] 收到剪贴板历史更新通知:', data)

      // 使用节流函数重新加载剪贴板历史，避免频繁更新导致的性能问题
      throttledLoadClipboardHistory()
    })

    // 监听标签数据更新通知
    removeTagsUpdateListener = onSimple('tags-updated', (data) => {
      console.log('[前端] 收到标签数据更新通知:', data)

      // 重新加载标签数据
      tagManagement.reloadTags().catch(error => {
        console.warn('[前端] 重新加载标签数据失败:', error)
      })
    })

    console.log('[前端] Overlay 组件初始化完成')
    // 注意：关闭事件监听器将在载入动画完成后通过 setupCloseEventListeners() 设置
  } catch (error) {
    console.error('[前端] Overlay 组件初始化失败:', error)

    // 初始化失败时的错误处理
    throw new OverlayError(
      'Overlay 组件初始化失败',
      OverlayErrorCode.INVALID_STATE,
      { originalError: error }
    )
  }
})

/**
 * 组件卸载时的清理逻辑
 * 清理所有事件监听器、定时器和资源，防止内存泄漏
 */
onUnmounted(() => {
  try {
    console.log('[前端] 开始清理 Overlay 组件资源')

    // 清理 IPC 监听器
    if (removeSlideInListener) {
      removeSlideInListener()
      removeSlideInListener = null
      console.log('[前端] 滑入动画监听器已清理')
    }

    // 清理关闭事件监听器（如果已设置）
    if (removeCloseListener) {
      removeCloseListener()
      removeCloseListener = null
      isCloseListenerSetup = false
      console.log('[前端] 关闭事件监听器已清理')
    }

    // 清理剪贴板更新监听器
    if (removeClipboardUpdateListener) {
      removeClipboardUpdateListener()
      removeClipboardUpdateListener = null
      console.log('[前端] 剪贴板更新监听器已清理')
    }

    // 清理节流函数
    try {
      cancelThrottledLoad()
      console.log('[前端] 节流函数已清理')
    } catch (error) {
      console.warn('[前端] 清理节流函数失败:', error)
    }

    // 清理标签更新监听器
    if (removeTagsUpdateListener) {
      removeTagsUpdateListener()
      removeTagsUpdateListener = null
      console.log('[前端] 标签更新监听器已清理')
    }

    // 清理键盘事件监听器
    try {
      document.removeEventListener('keydown', keyboardNavigation.handleKeyNavigation)
      console.log('[前端] 键盘事件监听器已清理')
    } catch (error) {
      console.warn('[前端] 清理键盘事件监听器失败:', error)
    }

    // 清理滚轮事件监听器
    try {
      document.removeEventListener('wheel', scrollManagement.handleWheel)
      console.log('[前端] 滚轮事件监听器已清理')
    } catch (error) {
      console.warn('[前端] 清理滚轮事件监听器失败:', error)
    }

    // 清理搜索相关资源
    try {
      searchLogic.cleanup()
      console.log('[前端] 搜索逻辑资源已清理')
    } catch (error) {
      console.warn('[前端] 清理搜索逻辑资源失败:', error)
    }

    // 清理 smooth-scrollbar 实例
    try {
      scrollManagement.destroyScrollbar()
      console.log('[前端] smooth-scrollbar 实例已清理')
    } catch (error) {
      console.warn('[前端] 清理 smooth-scrollbar 实例失败:', error)
    }

    console.log('[前端] Overlay 组件资源清理完成')
  } catch (error) {
    console.error('[前端] Overlay 组件资源清理失败:', error)
    // 清理失败不应该阻止组件卸载，只记录错误
  }
})
</script>

<template>
  <div class="overlay-container">
    <div ref="overlayContentRef" class="overlay-content">
      <!-- 顶部工具栏 -->
      <div class="top-toolbar">
        <!-- 搜索图标 -->
        <button class="toolbar-icon search-icon" @click="handleToggleSearch">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="11" cy="11" r="8"></circle>
            <path d="m21 21-4.35-4.35"></path>
          </svg>
        </button>

        <!-- 搜索输入框 -->
        <div ref="searchContainerRef" v-show="searchAnimation.showSearchContainer" class="search-container">
          <input type="text" class="search-input" placeholder="Search clipboard history..."
            :value="searchLogic.searchText.value" @input="searchLogic.handleSearchInput"
            @keydown.escape="handleSearchEscape" />
        </div>

        <!-- 标签组 -->
        <div ref="tabsContainerRef" v-show="searchAnimation.showTabsContainer" class="tabs-container">
          <div v-for="tab in tagManagement.tabs.value" :key="tab.id" class="tab" :class="{
            active: tab.active,
            'has-count': tab.count > 0,
            'switching': tagManagement.isTagSwitching.value && tab.active
          }" tabindex="0" role="button"
            :aria-label="`Switch to ${tab.label} tag${tab.count > 0 ? ` (${tab.count} items)` : ''}`"
            @click="handleTagSwitch(tab.id)" @keydown.enter="handleTagSwitch(tab.id)"
            @keydown.space.prevent="handleTagSwitch(tab.id)">
            <span class="tab-dot" :style="{ backgroundColor: tab.color }"></span>
            <span class="tab-label">{{ tab.label }}</span>
            <span v-if="tab.count > 0" class="tab-count">{{ tab.count }}</span>
            <!-- 加载指示器 -->
            <div v-if="tagManagement.isTagSwitching.value && tab.active" class="tab-loading">
              <div class="loading-spinner"></div>
            </div>
          </div>
        </div>

        <!-- 右侧按钮组 -->
        <div class="toolbar-buttons">
          <!-- 加号按钮 -->
          <button class="toolbar-icon add-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19"></line>
              <line x1="5" y1="12" x2="19" y2="12"></line>
            </svg>
          </button>

          <!-- 菜单按钮 -->
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
              :is-selected="index === selectedIndex" @click="handleClipboardItemClick" />
          </transition-group>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
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

/* 顶部工具栏 */
.top-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 24px;
  background: #fef9e6;
  border-bottom: 1px solid #f0f0f0;
  backdrop-filter: blur(20px);
  height: 56px;
  /* 固定高度 */
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



.search-container {
  overflow: hidden;
  white-space: nowrap;
  transform-origin: left center;
  /* 初始状态为隐藏 */
  width: 0px;
  opacity: 0;
  transform: scaleX(0);
  padding-left: 0px;
  padding-right: 0px;
}

.tabs-container {
  display: flex;
  gap: 8px;
  flex: 1;
  overflow-x: auto;
  padding: 4px 8px;
  transform-origin: left center;
  /* 初始状态为显示 */
  opacity: 1;
  transform: scaleX(1);
  max-width: 100%;
  align-items: center;
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

/* 标签hover效果 */
.tab:hover:not(.switching) {
  background: rgba(255, 255, 255, 0.8);
  border-color: rgba(0, 0, 0, 0.08);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.06);
}

.tab:hover:not(.switching) .tab-dot {
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.6);
}

.tab:hover:not(.switching) .tab-label {
  color: #1f2937 !important;
}

.tab:hover:not(.switching) .tab-count {
  background: rgba(0, 0, 0, 0.1);
  color: #1f2937;
}

/* 标签焦点状态 */
.tab:focus {
  outline: none;
  background: rgba(255, 255, 255, 0.9);
  border-color: #3b82f6;
  box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.08);
}

.tab:focus .tab-label {
  color: #1f2937;
}

/* 活跃状态样式 */
.tab.active {
  background: rgba(255, 255, 255, 0.95);
  border-color: rgba(0, 0, 0, 0.12);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.08);
}

.tab.active .tab-dot {
  box-shadow: 0 0 0 2px rgba(255, 255, 255, 0.7);
}

.tab.active .tab-label {
  color: #1f2937;
}

.tab.active .tab-count {
  background: rgba(0, 0, 0, 0.1);
  color: #1f2937;
}

/* 非活跃状态样式 */
.tab:not(.active) .tab-label {
  color: #6b7280;
}

.tab:not(.active) .tab-count {
  background: rgba(107, 114, 128, 0.2);
  color: #6b7280;
}

.tab-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.tab-label {
  font-size: 14px;
  font-weight: 600;
  /* 固定字体粗细，避免宽度变化 */
  user-select: none;
}

.tab-count {
  font-size: 11px;
  font-weight: 600;
  /* 固定字体粗细，避免宽度变化 */
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
}

/* 标签切换状态 */
.tab.switching {
  pointer-events: none;
  opacity: 0.7;
}

.tab.switching .tab-dot {
  animation: pulse 1.5s ease-in-out infinite;
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

/* 动画定义 */
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@keyframes pulse {

  0%,
  100% {
    opacity: 1;
  }

  50% {
    opacity: 0.6;
  }
}

/* 标签组进入动画 */

@keyframes slideInFromLeft {
  from {
    opacity: 0;
    transform: translateX(-20px);
  }

  to {
    opacity: 1;
    transform: translateX(0);
  }
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

/* 高对比度模式支持 */
@media (prefers-contrast: high) {
  .tab {
    border-width: 2px;
  }

  .tab:hover:not(.switching) {
    border-color: #000;
  }

  .tab.active {
    border-color: #000;
  }

  .tab:focus {
    border-color: #0066cc;
    box-shadow: 0 0 0 2px rgba(0, 102, 204, 0.2);
  }
}

/* 减少动画偏好支持 */
@media (prefers-reduced-motion: reduce) {

  .tab,
  .tab-dot,
  .tab-label,
  .tab-count {
    transition: none;
    animation: none;
  }
}

/* 主体区域 */
.overlay-body {
  flex: 1;
  /* padding: 24px; */
  overflow-y: auto;
  background: #f9f9f9;
}



/* 空状态 */
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

/* 剪贴板横向滚动布局 */
.clipboard-scroll-container {
  padding-top: 15px;
  padding-left: 20px;
  padding-bottom: 15px;
  /* smooth-scrollbar 会处理滚动，移除原生滚动设置 */
  overflow: hidden;
  /* 为 smooth-scrollbar 提供足够的高度 */
  /* height: calc(100vh - 56px - 30px); 减去工具栏高度和内边距 */
}

:global(.clipboard-scroll-container .scroll-content) {
  transition: transform 0.4s ease;

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



/* 滚动条样式 */
.overlay-body::-webkit-scrollbar {
  width: 6px;
}

.overlay-body::-webkit-scrollbar-track {
  background: rgba(156, 163, 175, 0.1);
  border-radius: 3px;
}

.overlay-body::-webkit-scrollbar-thumb {
  background: rgba(156, 163, 175, 0.3);
  border-radius: 3px;
}

.overlay-body::-webkit-scrollbar-thumb:hover {
  background: rgba(156, 163, 175, 0.5);
}

.tabs-container::-webkit-scrollbar {
  height: 0;
}

/* smooth-scrollbar 样式覆盖 - 确保滚动条完全隐藏 */
.clipboard-scroll-container :deep(.scrollbar-track) {
  display: none !important;
}

.clipboard-scroll-container :deep(.scrollbar-thumb) {
  display: none !important;
}

.clipboard-scroll-container :deep(.scrollbar-track-x),
.clipboard-scroll-container :deep(.scrollbar-track-y) {
  display: none !important;
}

.clipboard-scroll-container :deep(.scrollbar-thumb-x),
.clipboard-scroll-container :deep(.scrollbar-thumb-y) {
  display: none !important;
}
</style>
