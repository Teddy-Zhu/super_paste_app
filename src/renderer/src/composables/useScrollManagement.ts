/**
 * 滚动管理 Composable
 * 管理剪贴板历史的滚动行为和触摸板手势
 */

import { ref, nextTick, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import type { ScrollManager } from '@renderer/types/overlay'
import { AnimationState } from './useOverlayAnimation'
import { useThrottle, useRafThrottle } from './usePerformanceOptimization'

export function useScrollManagement(
  clipboardScrollRef: Ref<HTMLElement | undefined>,
  clipboardHistory: Ref<ClipboardItem[]>,
  selectedIndex: Ref<number>,
  currentAnimationState: Ref<AnimationState>
): ScrollManager {
  // 滚动节流控制
  const isScrolling = ref(false)

  /**
   * 将选中的元素滚动到可视区域（使用 RAF 优化）
   */
  const [scrollToSelectedItem] = useRafThrottle(() => {
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
  })

  /**
   * 处理鼠标滚轮和触摸板事件（使用节流优化）
   */
  const [handleWheel] = useThrottle((event: WheelEvent) => {
    // 只有在活跃状态且有剪贴板历史时才响应滚轮事件
    if (currentAnimationState.value !== AnimationState.ACTIVE ||
      !clipboardScrollRef.value ||
      clipboardHistory.value.length === 0) {
      return
    }

    // 阻止默认的滚动行为
    event.preventDefault()

    // 检查是否为水平滚动（触摸板左右滑动）
    const isHorizontalScroll = Math.abs(event.deltaX) > Math.abs(event.deltaY)

    // 确定滚动方向和滚动量
    let scrollAmount = 0

    if (isHorizontalScroll && Math.abs(event.deltaX) > 3) {
      // 触摸板水平滑动 - 进行水平滚动
      scrollAmount = event.deltaX * 6 // 水平滚动速度调节，稍微降低敏感度
    } else {
      // 垂直滚动 - 也进行水平滚动（保持原有行为）
      scrollAmount = event.deltaY * 10
    }

    const currentScrollLeft = clipboardScrollRef.value.scrollLeft
    const maxScrollLeft = clipboardScrollRef.value.scrollWidth - clipboardScrollRef.value.clientWidth

    // 边界检查，确保不会超出滚动范围
    const targetScrollLeft = Math.max(0, Math.min(maxScrollLeft, currentScrollLeft + scrollAmount))

    // 如果已经到达边界，不执行滚动
    if (targetScrollLeft === currentScrollLeft) {
      return
    }

    // 使用平滑滚动
    clipboardScrollRef.value.scrollTo({
      left: targetScrollLeft,
      behavior: 'smooth'
    })
  }, 16) // 使用固定的节流延迟（约一帧的时间）

  /**
   * 确保选中项目可见（延迟执行）
   */
  function ensureSelectedItemVisible() {
    nextTick(() => {
      scrollToSelectedItem()
    })
  }

  return {
    isScrolling: readonly(isScrolling),
    scrollToSelectedItem,
    handleWheel,
    ensureSelectedItemVisible
  }
}
