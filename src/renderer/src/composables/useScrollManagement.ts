/**
 * 滚动管理 Composable
 * 管理剪贴板历史的滚动行为和触摸板手势
 * 使用 smooth-scrollbar 库提供流畅的横向滚动体验
 */

import { ref, nextTick, onUnmounted, type Ref } from 'vue'
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
  const [scrollToSelectedItem] = useRafThrottle(() => {
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
  const [handleWheel] = useThrottle((event: WheelEvent) => {
    // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
    if (currentAnimationState.value !== AnimationState.ACTIVE ||
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0) {
      return
    }

    // 如果 smooth-scrollbar 未初始化，先初始化
    if (!scrollbarInstance) {
      initializeScrollbar()
    }

    // 阻止默认的滚动行为
    event.preventDefault()

    // 检查是否为水平滚动（触摸板左右滑动）
    const isHorizontalScroll = Math.abs(event.deltaX) > Math.abs(event.deltaY)

    // 确定滚动方向和滚动量
    let scrollAmount = 0

    if (isHorizontalScroll && Math.abs(event.deltaX) > 3) {
      // 触摸板水平滑动 - 进行水平滚动
      scrollAmount = event.deltaX * 3 // 降低敏感度，smooth-scrollbar 会提供更好的滚动体验
    } else {
      // 垂直滚动 - 转换为水平滚动（保持原有行为）
      scrollAmount = event.deltaY * 5 // 降低敏感度
    }

    if (scrollbarInstance) {
      // 使用 smooth-scrollbar 进行滚动
      const currentScrollLeft = scrollbarInstance.scrollLeft
      const maxScrollLeft = scrollbarInstance.size.content.width - scrollbarInstance.size.container.width

      // 边界检查，确保不会超出滚动范围
      const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))

      // 如果已经到达边界，不执行滚动
      if (targetScrollLeft === currentScrollLeft) {
        return
      }

      // 使用 smooth-scrollbar 的平滑滚动，不使用动画时间以保持响应性
      scrollbarInstance.scrollTo(targetScrollLeft, 0, 0)
    } else {
      // 回退到原生滚动
      const container = clipboardScrollRef.value
      const currentScrollLeft = container.scrollLeft
      const maxScrollLeft = container.scrollWidth - container.clientWidth

      // 边界检查，确保不会超出滚动范围
      const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))

      // 如果已经到达边界，不执行滚动
      if (targetScrollLeft === currentScrollLeft) {
        return
      }

      // 使用原生平滑滚动
      container.scrollTo({
        left: targetScrollLeft,
        behavior: 'smooth'
      })
    }
  }, 16) // 使用固定的节流延迟（约一帧的时间）

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
