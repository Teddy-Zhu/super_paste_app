<script setup lang="ts">
import { nextTick } from 'vue'
import { send, onSimple } from '@renderer/api'
import { clipboard, tags } from '@renderer/api/renderer-api'
import type { ClipboardItem, ClipboardHistoryQueryOptions, TagGroup } from '@renderer/api/types'

const overlayContentRef = ref<HTMLElement>()
const clipboardScrollRef = ref<HTMLElement>()

// 剪贴板历史记录
const clipboardHistory = ref<ClipboardItem[]>([])
const selectedIndex = ref(0)

// 动画配置
const ANIMATION_DURATION = 200
const SLIDE_IN_EASING = 'cubic-bezier(0.25, 0.46, 0.45, 0.94)'
const SLIDE_OUT_EASING = 'cubic-bezier(0.55, 0.06, 0.68, 0.19)'

// 前端动画状态枚举
enum AnimationState {
  IDLE = 'IDLE',                    // 空闲状态
  SLIDING_IN = 'SLIDING_IN',        // 正在滑入
  ACTIVE = 'ACTIVE',                // 活跃状态（可交互）
  SLIDING_OUT = 'SLIDING_OUT'       // 正在滑出
}

// 当前动画状态
const currentAnimationState = ref<AnimationState>(AnimationState.IDLE)

// 标记载入动画是否完成（保持向后兼容）
const isSlideInComplete = ref(false)

// 存储监听器取消函数
let removeSlideInListener: (() => void) | null = null
let removeCloseListener: (() => void) | null = null
let removeClipboardUpdateListener: (() => void) | null = null
let removeTagsUpdateListener: (() => void) | null = null

/**
 * 设置动画状态
 */
function setAnimationState(newState: AnimationState): void {
  const oldState = currentAnimationState.value
  currentAnimationState.value = newState
  console.log(`[前端] 动画状态变更: ${oldState} -> ${newState}`)

  // 更新兼容性标记
  isSlideInComplete.value = newState === AnimationState.ACTIVE
}

/**
 * 检查是否可以开始关闭动画
 */
function canStartClosing(): boolean {
  return currentAnimationState.value === AnimationState.ACTIVE
}

/**
 * 检查是否正在进行动画
 */
function isAnimating(): boolean {
  return currentAnimationState.value === AnimationState.SLIDING_IN ||
    currentAnimationState.value === AnimationState.SLIDING_OUT
}

// 创建滑入动画
const createSlideInAnimation = () => {
  return useAnimate(overlayContentRef, [
    { transform: 'translateY(100%)' },
    { transform: 'translateY(0%)' }
  ], {
    duration: ANIMATION_DURATION,
    easing: SLIDE_IN_EASING,
    fill: 'forwards'
  })
}

// 创建滑出动画
const createSlideOutAnimation = () => {
  return useAnimate(overlayContentRef, [
    { transform: 'translateY(0%)' },
    { transform: 'translateY(100%)' }
  ], {
    duration: ANIMATION_DURATION,
    easing: SLIDE_OUT_EASING,
    fill: 'forwards'
  })
}

// 关闭窗口
const closeWindow = async () => {
  // 检查是否可以开始关闭
  if (!canStartClosing()) {
    console.log(`[前端] 当前状态为 ${currentAnimationState.value}，无法开始关闭`)
    return
  }

  console.log('[前端] 开始关闭窗口流程')

  // 设置状态为正在滑出
  setAnimationState(AnimationState.SLIDING_OUT)

  const { play: playSlideOut, playState } = createSlideOutAnimation()
  playSlideOut()

  // 等待动画完成后关闭窗口
  watch(playState, (newState) => {
    if (newState === 'finished') {
      console.log('[前端] 滑出动画完成，通知后端关闭窗口')
      resetState()
      send('close-overlay-window')
    }
  })
}

// 设置关闭事件监听器
const setupCloseEventListeners = () => {
  // 监听来自主进程的关闭窗口信号
  removeCloseListener = onSimple('trigger-close-window', handleCloseWindow)
}

// 获取剪贴板历史记录
const loadClipboardHistory = async (options?: ClipboardHistoryQueryOptions) => {
  try {
    // 使用 searchHistory 替代 getHistory，支持更多筛选选项
    const defaultOptions: ClipboardHistoryQueryOptions = {
      limit: 200, // 获取最近12条记录，适合网格布局
      orderBy: 'last_used',
      orderDirection: 'DESC'
    }

    const queryOptions = { ...defaultOptions, ...options }
    const history = await clipboard.searchHistory(queryOptions)

    // 直接更新数据
    clipboardHistory.value = history

    // 重置选中索引，但保持在有效范围内
    if (selectedIndex.value >= history.length) {
      selectedIndex.value = Math.max(0, history.length - 1)
    }

    // 数据加载完成后，确保选中的项目可见
    nextTick(() => {
      scrollToSelectedItem()
    })
  } catch (error) {
    console.error('获取剪贴板历史失败:', error)
    clipboardHistory.value = []
  }
}



// 还原剪贴板内容
const restoreClipboardContent = async (itemId: string) => {
  try {
    const success = await clipboard.restore(itemId)
    if (success) {
      console.log('剪贴板内容已还原')
      // 还原成功后关闭窗口
      closeWindow()
    } else {
      console.error('还原剪贴板失败')
    }
  } catch (error) {
    console.error('还原剪贴板失败:', error)
  }
}

// 处理键盘导航
const handleKeyNavigation = (event: KeyboardEvent) => {
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
          toggleSearch()
          break
        case 'ArrowDown':
          // 如果有剪贴板历史，从搜索框移动到第一个项目
          if (clipboardHistory.value.length > 0) {
            event.preventDefault()
            searchInput.blur()
            selectedIndex.value = 0
            isKeyboardNavigation = true
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
          toggleSearch()
        } else {
          closeWindow()
        }
        break
      default:
        // 检查是否为可输入字符
        if (isInputCharacter(event)) {
          handleInputCharacter(event)
        }
        break
    }
    return
  }

  // 有剪贴板历史时的完整键盘处理
  switch (event.key) {
    case 'ArrowLeft':
      event.preventDefault()
      isKeyboardNavigation = true
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
      isKeyboardNavigation = true
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
      isKeyboardNavigation = true
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
      isKeyboardNavigation = true
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
        isKeyboardNavigation = true
        selectedIndex.value = 0
      }
      break
    case 'End':
      // End 键：跳转到最后一个项目
      if (isCtrlOrCmd) {
        event.preventDefault()
        isKeyboardNavigation = true
        selectedIndex.value = clipboardHistory.value.length - 1
      }
      break
    case 'Enter':
      event.preventDefault()
      if (clipboardHistory.value[selectedIndex.value]) {
        restoreClipboardContent(clipboardHistory.value[selectedIndex.value].id)
      }
      break
    case 'Escape':
      event.preventDefault()
      if (isSearchVisible.value) {
        toggleSearch()
      } else {
        closeWindow()
      }
      break
    default:
      // 检查是否为可输入字符
      if (isInputCharacter(event)) {
        handleInputCharacter(event)
      }
      break
  }
}

// 处理输入字符
const handleInputCharacter = (event: KeyboardEvent) => {
  // 如果搜索框未完全打开（未打开或正在打开动画中），则缓存字符
  if (!isSearchVisible.value || isOpeningSearch.value || isSearchAnimating.value) {
    // 阻止默认行为，防止字符丢失
    event.preventDefault()

    // 将字符添加到缓冲区
    inputBuffer.value += event.key
    console.log('[前端] 字符已缓存到输入缓冲区:', inputBuffer.value)

    // 如果搜索框完全未打开，则开始打开
    if (!isSearchVisible.value && !isOpeningSearch.value) {
      toggleSearch(true) // 传入 true 表示使用缓冲区
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

        // 将光标移动到末尾
        nextTick(() => {
          if (searchInput) {
            searchInput.setSelectionRange(searchText.value.length, searchText.value.length)
          }
        })
      }
    }
    // 如果搜索框已有焦点，不做任何处理，让默认行为处理输入
  }
}

// 检查是否为可输入字符
const isInputCharacter = (event: KeyboardEvent): boolean => {
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



// 滚动节流控制
let isScrolling = false
// 标记是否由键盘导航触发的选择变化
let isKeyboardNavigation = false



// 将选中的元素滚动到可视区域
const scrollToSelectedItem = () => {
  if (!clipboardScrollRef.value || clipboardHistory.value.length === 0) return

  const container = clipboardScrollRef.value
  const cardWidth = 300 + 32 // 卡片宽度 + gap
  const selectedCardLeft = selectedIndex.value * cardWidth
  const selectedCardRight = selectedCardLeft + 300

  const containerScrollLeft = container.scrollLeft
  const containerWidth = container.clientWidth
  const containerScrollRight = containerScrollLeft + containerWidth

  // 检查选中的卡片是否在可视区域内
  if (selectedCardLeft < containerScrollLeft) {
    // 选中卡片在左侧外部，滚动到该卡片的左边缘
    container.scrollTo({
      left: Math.max(0, selectedCardLeft - 20), // 添加一些边距，确保不为负数
      behavior: 'smooth'
    })
  } else if (selectedCardRight > containerScrollRight) {
    // 选中卡片在右侧外部，滚动到该卡片的右边缘
    const maxScrollLeft = container.scrollWidth - container.clientWidth
    container.scrollTo({
      left: Math.min(maxScrollLeft, selectedCardRight - containerWidth + 20), // 添加一些边距，确保不超出最大值
      behavior: 'smooth'
    })
  }
}

// 处理鼠标滚轮和触摸板事件
const handleWheel = (event: WheelEvent) => {
  // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
  if (currentAnimationState.value !== AnimationState.ACTIVE ||
    !clipboardScrollRef.value ||
    clipboardHistory.value.length === 0) {
    return
  }

  // 阻止默认的滚动行为
  event.preventDefault()

  // 节流控制，防止过度频繁的滚动
  if (isScrolling) return

  isScrolling = true

  // 检查是否为水平滚动（触摸板左右滑动）
  const isHorizontalScroll = Math.abs(event.deltaX) > Math.abs(event.deltaY)

  // 确定滚动方向和滚动量
  let scrollAmount = 0
  // let scrollDirection = ''

  if (isHorizontalScroll && Math.abs(event.deltaX) > 3) {
    // 触摸板水平滑动 - 进行水平滚动
    scrollAmount = event.deltaX * 6 // 水平滚动速度调节，稍微降低敏感度
    // scrollDirection = event.deltaX > 0 ? '向右' : '向左'
    // console.log('[前端] 触摸板水平滑动:', { deltaX: event.deltaX, deltaY: event.deltaY, scrollDirection })
  } else {
    // 垂直滚动 - 也进行水平滚动（保持原有行为）
    scrollAmount = event.deltaY * 10
    // scrollDirection = event.deltaY > 0 ? '向下转右' : '向上转左'
  }

  const currentScrollLeft = clipboardScrollRef.value.scrollLeft
  const maxScrollLeft = clipboardScrollRef.value.scrollWidth - clipboardScrollRef.value.clientWidth

  // 边界检查，确保不会超出滚动范围
  const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))

  // 如果已经到达边界，不执行滚动
  if (targetScrollLeft === currentScrollLeft) {
    isScrolling = false
    return
  }

  // 使用平滑滚动
  clipboardScrollRef.value.scrollTo({
    left: targetScrollLeft,
    behavior: 'smooth'
  })

  if (isHorizontalScroll) {
    // console.log('[前端] 水平滚动执行:', {
    //   scrollAmount,
    //   currentScrollLeft,
    //   targetScrollLeft,
    //   scrollDirection
    // })
  }

  // 重置节流标志，水平滚动使用更短的间隔以提高响应性
  const throttleDelay = isHorizontalScroll ? 10 : 30
  setTimeout(() => {
    isScrolling = false
  }, throttleDelay)
}



// 搜索状态
const isSearchVisible = ref(false)
const searchText = ref('')
const searchTimeout = ref<NodeJS.Timeout | null>(null)
const isSearching = ref(false) // 标记是否正在搜索中

// 输入缓冲区，用于在搜索框打开过程中缓存输入的字符
const inputBuffer = ref('')
const isOpeningSearch = ref(false) // 标记是否正在打开搜索框

// 搜索框和标签组动画状态控制
const isSearchAnimating = ref(false)

// 搜索框和标签组的DOM引用
const searchContainerRef = ref<HTMLElement>()
const tabsContainerRef = ref<HTMLElement>()

// 计算属性：控制元素可见性，在动画期间两个元素都可见
const showSearchContainer = computed(() => {
  return isSearchVisible.value || isSearchAnimating.value
})

const showTabsContainer = computed(() => {
  return !isSearchVisible.value || isSearchAnimating.value
})

// 搜索框展开动画
const createSearchExpandAnimation = () => {
  return useAnimate(searchContainerRef, [
    {
      width: '0px',
      opacity: 0,
      transform: 'scaleX(0)',
      paddingLeft: '0px',
      paddingRight: '0px'
    },
    {
      width: '600px',
      opacity: 1,
      transform: 'scaleX(1)',
      paddingLeft: '8px',
      paddingRight: '8px'
    }
  ], {
    duration: 300,
    easing: 'cubic-bezier(0.4, 0, 0.2, 1)',
    fill: 'forwards'
  })
}

// 搜索框缩回动画
const createSearchCollapseAnimation = () => {
  return useAnimate(searchContainerRef, [
    {
      width: '400px',
      opacity: 1,
      transform: 'scaleX(1)',
      paddingLeft: '8px',
      paddingRight: '8px'
    },
    {
      width: '0px',
      opacity: 0,
      transform: 'scaleX(0)',
      paddingLeft: '0px',
      paddingRight: '0px'
    }
  ], {
    duration: 300,
    easing: 'cubic-bezier(0.4, 0, 0.2, 1)',
    fill: 'forwards'
  })
}

// 标签组隐藏动画
const createTabsHideAnimation = () => {
  return useAnimate(tabsContainerRef, [
    {
      opacity: 1,
      transform: 'scaleX(1)',
      maxWidth: '100%'
    },
    {
      opacity: 0,
      transform: 'scaleX(0.8)',
      maxWidth: '0px'
    }
  ], {
    duration: 300,
    easing: 'cubic-bezier(0.4, 0, 0.2, 1)',
    fill: 'forwards'
  })
}

// 标签组显示动画
const createTabsShowAnimation = () => {
  return useAnimate(tabsContainerRef, [
    {
      opacity: 0,
      transform: 'scaleX(0.8)',
      maxWidth: '0px'
    },
    {
      opacity: 1,
      transform: 'scaleX(1)',
      maxWidth: '100%'
    }
  ], {
    duration: 300,
    easing: 'cubic-bezier(0.4, 0, 0.2, 1)',
    fill: 'forwards'
  })
}

// 切换搜索框显示状态
const toggleSearch = async (withBuffer = false) => {
  // 防止重复触发动画
  if (isSearchAnimating.value) {
    console.log('[前端] 动画进行中，忽略切换请求')
    return
  }

  if (isSearchVisible.value) {
    // 隐藏搜索框，显示标签组 - 同时进行动画
    // console.log('[前端] 开始隐藏搜索框，显示标签组')

    // 设置动画状态
    isSearchAnimating.value = true

    // 同时播放搜索框缩回动画和标签组显示动画
    const { play: playSearchCollapse, playState: searchCollapseState } = createSearchCollapseAnimation()
    const { play: playTabsShow, playState: tabsShowState } = createTabsShowAnimation()

    playSearchCollapse()
    playTabsShow()

    // 等待两个动画都完成
    await Promise.all([
      new Promise<void>((resolve) => {
        watch(searchCollapseState, (newState) => {
          if (newState === 'finished') {
            // console.log('[前端] 搜索框缩回动画完成')
            resolve()
          }
        })
      }),
      new Promise<void>((resolve) => {
        watch(tabsShowState, (newState) => {
          if (newState === 'finished') {
            // console.log('[前端] 标签组显示动画完成')
            resolve()
          }
        })
      })
    ])

    // 动画完成后设置状态
    isSearchVisible.value = false
    isSearchAnimating.value = false
  } else {
    // 显示搜索框，隐藏标签组 - 同时进行动画
    // console.log('[前端] 开始显示搜索框，隐藏标签组')

    // 标记正在打开搜索框
    isOpeningSearch.value = true

    // 设置动画状态
    isSearchAnimating.value = true

    // 先设置搜索框为显示状态，让DOM元素可见以便动画
    isSearchVisible.value = true

    // 等待DOM更新
    await nextTick()

    // 同时播放标签组隐藏动画和搜索框展开动画
    const { play: playTabsHide, playState: tabsHideState } = createTabsHideAnimation()
    const { play: playSearchExpand, playState: searchExpandState } = createSearchExpandAnimation()

    playTabsHide()
    playSearchExpand()

    // 等待两个动画都完成
    await Promise.all([
      new Promise<void>((resolve) => {
        watch(tabsHideState, (newState) => {
          if (newState === 'finished') {
            console.log('[前端] 标签组隐藏动画完成')
            resolve()
          }
        })
      }),
      new Promise<void>((resolve) => {
        watch(searchExpandState, (newState) => {
          if (newState === 'finished') {
            console.log('[前端] 搜索框展开动画完成')
            resolve()
          }
        })
      })
    ])

    // 动画完成后设置状态
    isSearchAnimating.value = false
    isOpeningSearch.value = false

    // 如果有缓冲的输入，应用到搜索框
    if (withBuffer && inputBuffer.value) {
      // 追加缓冲区内容到现有搜索文本
      searchText.value += inputBuffer.value
      inputBuffer.value = '' // 清空缓冲区
      // console.log('[前端] 缓冲区内容已应用到搜索框:', searchText.value)
      // 触发搜索
      debouncedSearch(searchText.value)
    }

    // 聚焦搜索框
    await nextTick()
    const searchInput = document.querySelector('.search-input') as HTMLInputElement
    if (searchInput) {
      searchInput.focus()
      // 将光标移动到末尾
      if (searchText.value) {
        searchInput.setSelectionRange(searchText.value.length, searchText.value.length)
      }
    }
  }
}

// 执行搜索
const performSearch = async (text: string) => {
  try {
    isSearching.value = true

    if (text.trim()) {
      // 有搜索文本时进行搜索
      await loadClipboardHistory({
        searchText: text.trim(),
        limit: 50, // 搜索时可以显示更多结果
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })
    } else {
      // 搜索文本为空时显示默认历史记录
      await loadClipboardHistory()
    }
  } catch (error) {
    console.error('搜索失败:', error)
  } finally {
    isSearching.value = false
  }
}

// 防抖搜索
const debouncedSearch = (text: string) => {
  // 清除之前的搜索定时器
  if (searchTimeout.value) {
    clearTimeout(searchTimeout.value)
  }

  // 设置新的搜索定时器
  searchTimeout.value = setTimeout(() => {
    performSearch(text)
  }, 300) // 300ms 防抖延迟
}

// 处理搜索输入
const handleSearchInput = (event: Event) => {
  const target = event.target as HTMLInputElement
  // 由于使用了 v-model，searchText.value 会自动更新
  // 这里只需要触发防抖搜索
  debouncedSearch(target.value)
}

// 包装函数：处理搜索框切换（用于模板）
const handleToggleSearch = () => {
  toggleSearch(false)
}

// 包装函数：处理搜索框ESC键（用于模板）
const handleSearchEscape = () => {
  toggleSearch(false)
}

// 标签数据
const allTags = ref<TagGroup[]>([])
const activeTagId = ref<string>('all') // 默认选中'all'标签

// 标签统计数据
const tagStats = ref<Record<string, number>>({})

// 计算属性：生成标签组件数据，包含'all'标签和实际标签
const tabs = computed(() => {
  const allTab = {
    id: 'all',
    label: 'All',
    active: activeTagId.value === 'all',
    color: '#10b981',
    count: 0 // All标签不显示数量
  }
  const tagTabs = allTags.value.map(tag => ({
    id: tag.id,
    label: tag.name,
    active: activeTagId.value === tag.id,
    color: tag.color,
    count: tagStats.value[tag.id] || 0
  }))
  return [allTab, ...tagTabs]
})

// 加载所有标签
const loadAllTags = async () => {
  try {
    const tagList = await tags.getAllTags()
    allTags.value = tagList
    console.log('[前端] 标签数据加载完成:', tagList)

    // 加载标签统计数据
    await loadTagStats()
  } catch (error) {
    console.error('[前端] 加载标签数据失败:', error)
    allTags.value = []
  }
}

// 加载标签统计数据
const loadTagStats = async () => {
  try {
    // 这里可以调用后端API获取每个标签的记录数量
    // 暂时使用模拟数据，实际应该从后端获取
    const stats: Record<string, number> = {}

    for (const tag of allTags.value) {
      // 模拟统计数据，实际应该调用后端API
      stats[tag.id] = Math.floor(Math.random() * 20) + 1
    }

    tagStats.value = stats
    // console.log('[前端] 标签统计数据加载完成:', stats)
  } catch (error) {
    console.error('[前端] 加载标签统计数据失败:', error)
    tagStats.value = {}
  }
}

// 标签切换加载状态
const isTagSwitching = ref(false)

// 切换标签
const switchTag = async (tagId: string) => {
  if (activeTagId.value === tagId || isTagSwitching.value) return

  isTagSwitching.value = true
  activeTagId.value = tagId
  console.log('[前端] 切换到标签:', tagId)

  try {
    // 根据选中的标签重新加载剪贴板历史
    if (tagId === 'all') {
      // 显示所有记录
      await loadClipboardHistory()
    } else {
      // 按标签筛选
      await loadClipboardHistory({
        tagIds: [tagId],
        limit: 50,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })
    }
  } catch (error) {
    console.error('[前端] 切换标签失败:', error)
  } finally {
    isTagSwitching.value = false
  }
}

// 触发滑入动画
const triggerSlideInAnimation = async () => {
  console.log('[前端] 收到触发滑入动画信号')

  // 检查当前状态，防止重复触发
  if (isAnimating()) {
    console.log(`[前端] 正在进行动画，忽略滑入请求，当前状态: ${currentAnimationState.value}`)
    return
  }

  console.log('[前端] 开始滑入动画流程')

  // 重置搜索状态：如果搜索框无文字，默认关闭搜索框
  if (!searchText.value.trim()) {
    console.log('[前端] 搜索框无文字，重置为关闭状态')
    isSearchVisible.value = false
    isSearchAnimating.value = false

    // 重置搜索框样式到初始隐藏状态
    nextTick(() => {
      if (searchContainerRef.value) {
        searchContainerRef.value.style.width = '0px'
        searchContainerRef.value.style.opacity = '0'
        searchContainerRef.value.style.transform = 'scaleX(0)'
        searchContainerRef.value.style.paddingLeft = '0px'
        searchContainerRef.value.style.paddingRight = '0px'
      }

      // 重置标签组样式到初始显示状态
      if (tabsContainerRef.value) {
        tabsContainerRef.value.style.opacity = '1'
        tabsContainerRef.value.style.transform = 'scaleX(1)'
        tabsContainerRef.value.style.maxWidth = '100%'
      }
    })
  }

  // 清空输入缓冲区和相关状态
  inputBuffer.value = ''
  isOpeningSearch.value = false

  // 设置状态为正在滑入
  setAnimationState(AnimationState.SLIDING_IN)

  // 开始滑入动画
  const { play: playSlideIn, playState } = createSlideInAnimation()
  playSlideIn()

  // 等待滑入动画完成
  await new Promise<void>((resolve) => {
    watch(playState, (newState) => {
      if (newState === 'finished') {
        console.log('[前端] 滑入动画完成')
        resolve()
      }
    })
  })

  console.log('[前端] 设置为活跃状态')
  // 设置状态为活跃（可交互）
  setAnimationState(AnimationState.ACTIVE)

  // 载入动画完成后才设置关闭事件监听器
  setupCloseEventListeners()

  // 通知后端动画完成
  send('open-overlay-finished')
}

const resetState = () => {
  console.log('[前端] 重置overlay状态')

  // 重置动画状态为空闲
  setAnimationState(AnimationState.IDLE)

  // 清理之前的关闭事件监听器
  if (removeCloseListener) {
    removeCloseListener()
    removeCloseListener = null
  }

  // 重置元素位置到初始状态
  if (overlayContentRef.value) {
    overlayContentRef.value.style.transform = 'translateY(100%)'
  }
}

// 处理主进程发送的关闭窗口事件
const handleCloseWindow = () => {
  console.log(`[前端] 收到关闭窗口信号，当前状态: ${currentAnimationState.value}`)

  // 只有在活跃状态时才响应关闭事件
  if (currentAnimationState.value === AnimationState.ACTIVE) {
    // 检查搜索框状态
    if (isSearchVisible.value && searchText.value.trim()) {
      // 搜索框为展示状态且有值，清空搜索框的值并返回
      console.log('[前端] 搜索框有值，清空搜索框并返回')
      searchText.value = ''
      // 触发搜索以重新加载默认数据
      performSearch('')
      return
    }

    closeWindow()
  } else {
    console.log(`[前端] 当前状态为 ${currentAnimationState.value}，忽略关闭请求`)
  }
}

// 监听 selectedIndex 变化，自动滚动到可视区域
watch(selectedIndex, () => {
  // 只有在键盘导航时才自动滚动
  if (isKeyboardNavigation) {
    // 延迟执行，确保 DOM 更新完成
    nextTick(() => {
      scrollToSelectedItem()
      // 重置标志
      isKeyboardNavigation = false
    })
  }
})

// 监听 searchText 变化，自动触发搜索（用于缓冲区应用时）
watch(searchText, (newValue) => {
  // 只有当搜索框可见且不是正在打开时才触发搜索
  if (isSearchVisible.value && !isOpeningSearch.value && newValue !== undefined) {
    debouncedSearch(newValue)
  }
})

onMounted(() => {
  // 监听来自主进程的动画触发信号
  removeSlideInListener = onSimple('trigger-slide-in-animation', () => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        triggerSlideInAnimation()
      })
    })
  })

  // 在组件挂载时就绑定键盘事件监听器
  document.addEventListener('keydown', handleKeyNavigation)

  // 添加滚轮事件监听器（支持触摸板手势）
  document.addEventListener('wheel', handleWheel, { passive: false })

  // 预加载剪贴板历史以提高首次显示速度
  loadClipboardHistory().catch(error => {
    console.warn('[前端] 预加载剪贴板历史失败:', error)
  })

  // 预加载标签数据
  loadAllTags().catch(error => {
    console.warn('[前端] 预加载标签数据失败:', error)
  })

  // 初始化搜索框和标签组的状态
  nextTick(() => {
    // 确保搜索框初始为隐藏状态
    if (searchContainerRef.value) {
      searchContainerRef.value.style.width = '0px'
      searchContainerRef.value.style.opacity = '0'
      searchContainerRef.value.style.transform = 'scaleX(0)'
      searchContainerRef.value.style.paddingLeft = '0px'
      searchContainerRef.value.style.paddingRight = '0px'
    }

    // 确保标签组初始为显示状态
    if (tabsContainerRef.value) {
      tabsContainerRef.value.style.opacity = '1'
      tabsContainerRef.value.style.transform = 'scaleX(1)'
      tabsContainerRef.value.style.maxWidth = '100%'
    }
  })

  // 监听剪贴板历史更新通知
  removeClipboardUpdateListener = onSimple('clipboard-history-updated', (data) => {
    console.log('[前端] 收到剪贴板历史更新通知:', data)

    // 重新加载剪贴板历史
    loadClipboardHistory().catch(error => {
      console.warn('[前端] 重新加载剪贴板历史失败:', error)
    })
  })

  // 监听标签数据更新通知
  removeTagsUpdateListener = onSimple('tags-updated', (data) => {
    console.log('[前端] 收到标签数据更新通知:', data)

    // 重新加载标签数据
    loadAllTags().catch(error => {
      console.warn('[前端] 重新加载标签数据失败:', error)
    })
  })

  // 注意：关闭事件监听器将在载入动画完成后通过 setupCloseEventListeners() 设置
})

onUnmounted(() => {
  // 清理 IPC 监听器
  if (removeSlideInListener) {
    removeSlideInListener()
  }

  // 清理关闭事件监听器（如果已设置）
  if (isSlideInComplete.value && removeCloseListener) {
    removeCloseListener()
  }

  // 清理剪贴板更新监听器
  if (removeClipboardUpdateListener) {
    removeClipboardUpdateListener()
  }

  // 清理标签更新监听器
  if (removeTagsUpdateListener) {
    removeTagsUpdateListener()
  }

  // 清理键盘事件监听器
  document.removeEventListener('keydown', handleKeyNavigation)

  // 清理滚轮事件监听器
  document.removeEventListener('wheel', handleWheel)

  // 清理搜索定时器
  if (searchTimeout.value) {
    clearTimeout(searchTimeout.value)
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
        <div ref="searchContainerRef" v-show="showSearchContainer" class="search-container">
          <input type="text" class="search-input" placeholder="Search clipboard history..." v-model="searchText"
            @input="handleSearchInput" @keydown.escape="handleSearchEscape" />
        </div>

        <!-- 标签组 -->
        <div ref="tabsContainerRef" v-show="showTabsContainer" class="tabs-container">
          <div v-for="tab in tabs" :key="tab.id" class="tab" :class="{
            active: tab.active,
            'has-count': tab.count > 0,
            'switching': isTagSwitching && tab.active
          }" tabindex="0" role="button"
            :aria-label="`Switch to ${tab.label} tag${tab.count > 0 ? ` (${tab.count} items)` : ''}`"
            @click="switchTag(tab.id)" @keydown.enter="switchTag(tab.id)" @keydown.space.prevent="switchTag(tab.id)">
            <span class="tab-dot" :style="{ backgroundColor: tab.color }"></span>
            <span class="tab-label">{{ tab.label }}</span>
            <span v-if="tab.count > 0" class="tab-count">{{ tab.count }}</span>
            <!-- 加载指示器 -->
            <div v-if="isTagSwitching && tab.active" class="tab-loading">
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
              :is-selected="index === selectedIndex" @click="restoreClipboardContent" />
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
  padding: 8px 12px;
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  font-size: 14px;
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
  overflow-x: auto;
  overflow-y: hidden;
  scroll-behavior: smooth;
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

.clipboard-scroll-container::-webkit-scrollbar {
  height: 0px;
}

.clipboard-scroll-container::-webkit-scrollbar-track {
  background: #f1f3f4;
  border-radius: 3px;
}

.clipboard-scroll-container::-webkit-scrollbar-thumb {
  background: #c1c8cd;
  border-radius: 3px;
}

.clipboard-scroll-container::-webkit-scrollbar-thumb:hover {
  background: #a8b1b8;
}
</style>
