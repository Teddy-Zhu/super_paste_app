/**
 * Overlay 动画管理 Composable
 * 管理 overlay 窗口的滑入滑出动画状态
 */

import { ref, type Ref } from 'vue'
import type {
  OverlayAnimationManager,
  AnimationConfig,
  AnimationControls
} from '@renderer/types/overlay'

// 前端动画状态枚举
export enum AnimationState {
  IDLE = 'IDLE',                    // 空闲状态
  SLIDING_IN = 'SLIDING_IN',        // 正在滑入
  ACTIVE = 'ACTIVE',                // 活跃状态（可交互）
  SLIDING_OUT = 'SLIDING_OUT'       // 正在滑出
}

// 动画配置常量
export const ANIMATION_CONFIG: AnimationConfig = {
  DURATION: 200,
  SLIDE_IN_EASING: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
  SLIDE_OUT_EASING: 'cubic-bezier(0.55, 0.06, 0.68, 0.19)'
} as const

export function useOverlayAnimation(
  overlayContentRef: Ref<HTMLElement | undefined>
): OverlayAnimationManager {
  // 当前动画状态
  const currentAnimationState = ref<AnimationState>(AnimationState.IDLE)
  
  // 标记载入动画是否完成（保持向后兼容）
  const isSlideInComplete = ref(false)

  /**
   * 设置动画状态
   */
  function setAnimationState(newState: AnimationState): void {
    currentAnimationState.value = newState
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

  /**
   * 创建滑入动画
   */
  function createSlideInAnimation(): AnimationControls {
    return useAnimate(overlayContentRef, [
      { transform: 'translateY(100%)' },
      { transform: 'translateY(0%)' }
    ], {
      duration: ANIMATION_CONFIG.DURATION,
      easing: ANIMATION_CONFIG.SLIDE_IN_EASING,
      fill: 'forwards'
    })
  }

  /**
   * 创建滑出动画
   */
  function createSlideOutAnimation(): AnimationControls {
    return useAnimate(overlayContentRef, [
      { transform: 'translateY(0%)' },
      { transform: 'translateY(100%)' }
    ], {
      duration: ANIMATION_CONFIG.DURATION,
      easing: ANIMATION_CONFIG.SLIDE_OUT_EASING,
      fill: 'forwards'
    })
  }

  /**
   * 重置状态
   */
  function resetState() {
    console.log('[前端] 重置overlay状态')
    
    // 重置动画状态为空闲
    setAnimationState(AnimationState.IDLE)
    
    // 重置元素位置到初始状态
    if (overlayContentRef.value) {
      overlayContentRef.value.style.transform = 'translateY(100%)'
    }
  }

  return {
    // 状态
    currentAnimationState: readonly(currentAnimationState),
    isSlideInComplete: readonly(isSlideInComplete),
    
    // 方法
    setAnimationState,
    canStartClosing,
    isAnimating,
    createSlideInAnimation,
    createSlideOutAnimation,
    resetState
  }
}
