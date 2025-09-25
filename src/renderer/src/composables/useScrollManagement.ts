/**
 * 滚动管理 Composable
 * 管理剪贴板历史的滚动行为和触摸板手势
 * 使用 smooth-scrollbar 库提供流畅的横向滚动体验
 */

import { ref, nextTick, onUnmounted, readonly, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import type { ScrollManager } from '@renderer/types/overlay'
import { AnimationState } from './useOverlayAnimation'
import { useThrottle, useRafThrottle } from './usePerformanceOptimization'
import Scrollbar from 'smooth-scrollbar'
import type { ScrollbarOptions } from 'smooth-scrollbar/interfaces'

export function useScrollManagement(
  clipboardScrollRef: Ref<HTMLElement | undefined>,
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  currentAnimationState: Ref<AnimationState>
): ScrollManager {
  // 滚动节流控制
  const isScrolling = ref(false)

  // smooth-scrollbar 实例
  let scrollbarInstance: Scrollbar | null = null

  /**
   * 初始化 smooth-scrollbar
   * 配置为仅支持横向滚动，隐藏滚动条
   */
  const initializeScrollbar = () => {
    if (!clipboardScrollRef.value || scrollbarInstance) return

    const options: Partial<ScrollbarOptions> = {
      damping: 0.1, // 滚动阻尼，值越小滚动越流畅
      renderByPixels: true, // 按像素渲染，提高性能
      alwaysShowTracks: false, // 不显示滚动轨道
      continuousScrolling: true // 启用连续滚动
    }

    try {
      scrollbarInstance = Scrollbar.init(clipboardScrollRef.value, options)

      // 隐藏滚动条
      if (scrollbarInstance) {
        const scrollbarElement = scrollbarInstance.containerEl
        const style = scrollbarElement.style
        style.setProperty('--scrollbar-track-color', 'transparent')
        style.setProperty('--scrollbar-thumb-color', 'transparent')

        // 添加自定义样式来完全隐藏滚动条
        const styleSheet = document.createElement('style')
        styleSheet.textContent = `
          .scrollbar-track {
            display: none !important;
          }
          .scrollbar-thumb {
            display: none !important;
          }
        `
        document.head.appendChild(styleSheet)
      }

      console.log('[滚动管理] smooth-scrollbar 初始化成功')
    } catch (error) {
      console.error('[滚动管理] smooth-scrollbar 初始化失败:', error)
      scrollbarInstance = null
    }
  }

  /**
   * 销毁 smooth-scrollbar 实例
   */
  const destroyScrollbar = () => {
    if (scrollbarInstance) {
      try {
        scrollbarInstance.destroy()
        scrollbarInstance = null
        console.log('[滚动管理] smooth-scrollbar 实例已销毁')
      } catch (error) {
        console.error('[滚动管理] 销毁 smooth-scrollbar 实例失败:', error)
      }
    }
  }

  /**
   * 将选中的元素滚动到可视区域（使用 smooth-scrollbar）
   */
  const scrollToSelectedItem = useRafThrottle(() => {
    if (!clipboardScrollRef.value || clipboardHistory.value.length === 0) return

    // 如果 smooth-scrollbar 未初始化，先初始化
    if (!scrollbarInstance) {
      initializeScrollbar()
    }

    const cardWidth = 300 + 32 // 卡片宽度 + gap
    const selectedCardLeft = selectedIndex.value * cardWidth
    const selectedCardRight = selectedCardLeft + 300

    let containerScrollLeft: number
    let containerWidth: number

    if (scrollbarInstance) {
      // 使用 smooth-scrollbar 的滚动位置
      containerScrollLeft = scrollbarInstance.scrollLeft
      containerWidth = scrollbarInstance.size.container.width
    } else {
      // 回退到原生滚动
      const container = clipboardScrollRef.value
      containerScrollLeft = container.scrollLeft
      containerWidth = container.clientWidth
    }

    const containerScrollRight = containerScrollLeft + containerWidth

    // 检查选中的卡片是否在可视区域内
    if (selectedCardLeft < containerScrollLeft) {
      // 选中卡片在左侧外部，滚动到该卡片的左边缘
      const targetLeft = Math.max(0, selectedCardLeft - 20)

      if (scrollbarInstance) {
        scrollbarInstance.scrollTo(targetLeft, 0, 300) // 300ms 动画时间
      } else {
        clipboardScrollRef.value?.scrollTo({
          left: targetLeft,
          behavior: 'smooth'
        })
      }
    } else if (selectedCardRight > containerScrollRight) {
      // 选中卡片在右侧外部，滚动到该卡片的右边缘
      let maxScrollLeft: number

      if (scrollbarInstance) {
        maxScrollLeft = scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
      } else {
        const container = clipboardScrollRef.value!
        maxScrollLeft = container.scrollWidth - container.clientWidth
      }

      const targetLeft = Math.min(maxScrollLeft, selectedCardRight - containerWidth + 20)

      if (scrollbarInstance) {
        scrollbarInstance.scrollTo(targetLeft, 0, 300) // 300ms 动画时间
      } else {
        clipboardScrollRef.value?.scrollTo({
          left: targetLeft,
          behavior: 'smooth'
        })
      }
    }
  })

  /**
   * 处理鼠标滚轮和触摸板事件（使用 smooth-scrollbar）
   */
  const handleWheel = useThrottle((event: WheelEvent) => {
    // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
    if (
      currentAnimationState.value !== AnimationState.ACTIVE ||
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0
    ) {
      return
    }

    // 如果 smooth-scrollbar 未初始化，先初始化
    if (!scrollbarInstance) {
      initializeScrollbar()
    }

    // 仅对触摸板（像素级滚动）且纵向滚动占主导时做处理：把纵向滚动转换为横向滚动
    const isPixelMode = event.deltaMode === 0
    const absDeltaX = Math.abs(event.deltaX)
    const absDeltaY = Math.abs(event.deltaY)

    if (!(isPixelMode && absDeltaY > absDeltaX && event.deltaY !== 0)) {
      // 不是需要转换的情况：不拦截，交给默认/原有逻辑（包括横向滚动）
      return
    }

    // 拦截默认的纵向滚动，改用横向滚动
    event.preventDefault()

    const scrollAmount = event.deltaY // 1:1 转换，不做额外放大/缩小

    if (scrollbarInstance) {
      const currentScrollLeft = scrollbarInstance.scrollLeft
      const maxScrollLeft =
        scrollbarInstance.size.content.width - scrollbarInstance.size.container.width
      const targetScrollLeft = Math.max(
        0,
        Math.min(maxScrollLeft, currentScrollLeft + scrollAmount)
      )
      if (targetScrollLeft !== currentScrollLeft) {
        // 立即更新位置，不使用额外动画
        scrollbarInstance.scrollTo(targetScrollLeft, 0, 0)
      }
    } else {
      const container = clipboardScrollRef.value
      const currentScrollLeft = container.scrollLeft
      const maxScrollLeft = container.scrollWidth - container.clientWidth
      const targetScrollLeft = Math.max(
        0,
        Math.min(maxScrollLeft, currentScrollLeft + scrollAmount)
      )
      if (targetScrollLeft !== currentScrollLeft) {
        container.scrollLeft = targetScrollLeft
      }
    }
  }, 16) // 固定节流（约一帧），仅用于轻量防抖

  /**
   * 确保选中项目可见（延迟执行）
   */
  function ensureSelectedItemVisible() {
    nextTick(() => {
      scrollToSelectedItem()
    })
  }

  // 组件卸载时清理 smooth-scrollbar 实例
  onUnmounted(() => {
    destroyScrollbar()
  })

  return {
    isScrolling: readonly(isScrolling),
    scrollToSelectedItem,
    handleWheel,
    ensureSelectedItemVisible,
    initializeScrollbar,
    destroyScrollbar
  }
}
