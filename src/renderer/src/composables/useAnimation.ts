/**
 * 统一动画管理 Composable
 * 合并原来的 overlay 动画和搜索动画管理，提供统一的动画接口
 */

import { ref, computed, nextTick, readonly, type Ref } from 'vue'
import { useAnimate } from '@vueuse/core'
import { useStateManager } from './useUtils'

// ==================== 类型定义 ====================

export enum AnimationState {
  IDLE = 'IDLE',
  SLIDING_IN = 'SLIDING_IN',
  ACTIVE = 'ACTIVE',
  SLIDING_OUT = 'SLIDING_OUT'
}

export interface AnimationConfig {
  readonly OVERLAY_DURATION: number
  readonly SEARCH_DURATION: number
  readonly SLIDE_IN_EASING: string
  readonly SLIDE_OUT_EASING: string
  readonly SEARCH_EASING: string
  readonly SEARCH_WIDTH: string
  readonly COLLAPSE_WIDTH: string
}

export interface AnimationControls {
  play: () => void
  playState: Ref<'idle' | 'running' | 'paused' | 'finished'>
}

export interface AnimationManager {
  // Overlay 动画状态
  readonly currentAnimationState: Ref<AnimationState>
  readonly isSlideInComplete: Ref<boolean>
  
  // 搜索动画状态
  readonly isSearchVisible: Ref<boolean>
  readonly isSearchAnimating: Ref<boolean>
  readonly isOpeningSearch: Ref<boolean>
  readonly showSearchContainer: Ref<boolean>
  readonly showTabsContainer: Ref<boolean>
  
  // Overlay 动画方法
  setAnimationState: (newState: AnimationState) => void
  canStartClosing: () => boolean
  isAnimating: () => boolean
  createSlideInAnimation: () => AnimationControls
  createSlideOutAnimation: () => AnimationControls
  resetOverlayState: () => void
  
  // 搜索动画方法
  createSearchExpandAnimation: () => AnimationControls
  createSearchCollapseAnimation: () => AnimationControls
  createTabsHideAnimation: () => AnimationControls
  createTabsShowAnimation: () => AnimationControls
  resetSearchStyles: () => void
  initializeStyles: () => void
  setSearchVisible: (visible: boolean) => void
  setSearchAnimating: (animating: boolean) => void
  setOpeningSearch: (opening: boolean) => void
}

// ==================== 动画配置 ====================

export const ANIMATION_CONFIG: AnimationConfig = {
  OVERLAY_DURATION: 200,
  SEARCH_DURATION: 250, // 减少搜索动画时长，让切换更快
  SLIDE_IN_EASING: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
  SLIDE_OUT_EASING: 'cubic-bezier(0.55, 0.06, 0.68, 0.19)',
  SEARCH_EASING: 'cubic-bezier(0.25, 0.1, 0.25, 1)', // 使用更流畅的缓动函数
  SEARCH_WIDTH: '600px',
  COLLAPSE_WIDTH: '400px'
} as const

// ==================== 主要函数 ====================

export function useAnimation(
  overlayContentRef: Ref<HTMLElement | undefined>,
  searchContainerRef: Ref<HTMLElement | undefined>,
  tabsContainerRef: Ref<HTMLElement | undefined>
): AnimationManager {
  
  // ==================== 状态管理 ====================
  
  // Overlay 动画状态
  const overlayStateManager = useStateManager<AnimationState>(AnimationState.IDLE)
  const isSlideInComplete = computed(() => 
    overlayStateManager.state.value === AnimationState.ACTIVE
  )
  
  // 搜索动画状态
  const isSearchVisible = ref(false)
  const isSearchAnimating = ref(false)
  const isOpeningSearch = ref(false)
  
  // 计算属性：控制元素可见性
  const showSearchContainer = computed(() => 
    isSearchVisible.value || isSearchAnimating.value
  )
  const showTabsContainer = computed(() => 
    !isSearchVisible.value || isSearchAnimating.value
  )
  
  // ==================== Overlay 动画方法 ====================
  
  const setAnimationState = (newState: AnimationState): void => {
    overlayStateManager.setState(newState)
  }
  
  const canStartClosing = (): boolean => {
    return overlayStateManager.state.value === AnimationState.ACTIVE
  }
  
  const isAnimating = (): boolean => {
    const state = overlayStateManager.state.value
    return state === AnimationState.SLIDING_IN || state === AnimationState.SLIDING_OUT
  }
  
  const createSlideInAnimation = (): AnimationControls => {
    return useAnimate(overlayContentRef, [
      { transform: 'translateY(100%)' },
      { transform: 'translateY(0%)' }
    ], {
      duration: ANIMATION_CONFIG.OVERLAY_DURATION,
      easing: ANIMATION_CONFIG.SLIDE_IN_EASING,
      fill: 'forwards'
    })
  }
  
  const createSlideOutAnimation = (): AnimationControls => {
    return useAnimate(overlayContentRef, [
      { transform: 'translateY(0%)' },
      { transform: 'translateY(100%)' }
    ], {
      duration: ANIMATION_CONFIG.OVERLAY_DURATION,
      easing: ANIMATION_CONFIG.SLIDE_OUT_EASING,
      fill: 'forwards'
    })
  }
  
  const resetOverlayState = () => {
    // console.log('[动画管理] 重置overlay状态')
    overlayStateManager.resetState()
    
    // 重置元素位置到初始状态
    if (overlayContentRef.value) {
      overlayContentRef.value.style.transform = 'translateY(100%)'
    }
  }
  
  // ==================== 搜索动画方法 ====================
  
  const createSearchExpandAnimation = (): AnimationControls => {
    return useAnimate(searchContainerRef, [
      {
        width: '0px',
        opacity: 0,
        transform: 'scaleX(0)',
        paddingLeft: '0px',
        paddingRight: '0px'
      },
      // 添加中间关键帧，让动画更自然
      {
        width: ANIMATION_CONFIG.SEARCH_WIDTH,
        opacity: 0.8,
        transform: 'scaleX(0.95)',
        paddingLeft: '6px',
        paddingRight: '6px',
        offset: 0.7
      },
      {
        width: ANIMATION_CONFIG.SEARCH_WIDTH,
        opacity: 1,
        transform: 'scaleX(1)',
        paddingLeft: '8px',
        paddingRight: '8px'
      }
    ], {
      duration: ANIMATION_CONFIG.SEARCH_DURATION,
      easing: ANIMATION_CONFIG.SEARCH_EASING,
      fill: 'forwards'
    })
  }

  const createSearchCollapseAnimation = (): AnimationControls => {
    return useAnimate(searchContainerRef, [
      {
        width: ANIMATION_CONFIG.COLLAPSE_WIDTH,
        opacity: 1,
        transform: 'scaleX(1)',
        paddingLeft: '8px',
        paddingRight: '8px'
      },
      // 添加中间关键帧，让收缩更平滑
      {
        width: '100px',
        opacity: 0.3,
        transform: 'scaleX(0.3)',
        paddingLeft: '2px',
        paddingRight: '2px',
        offset: 0.6
      },
      {
        width: '0px',
        opacity: 0,
        transform: 'scaleX(0)',
        paddingLeft: '0px',
        paddingRight: '0px'
      }
    ], {
      duration: ANIMATION_CONFIG.SEARCH_DURATION,
      easing: ANIMATION_CONFIG.SEARCH_EASING,
      fill: 'forwards'
    })
  }
  
  const createTabsHideAnimation = (): AnimationControls => {
    return useAnimate(tabsContainerRef, [
      {
        opacity: 1,
        transform: 'scaleX(1)',
        maxWidth: '100%'
      },
      // 添加中间关键帧，让隐藏更自然
      {
        opacity: 0.4,
        transform: 'scaleX(0.6)',
        maxWidth: '30%',
        offset: 0.5
      },
      {
        opacity: 0,
        transform: 'scaleX(0)',
        maxWidth: '0px'
      }
    ], {
      duration: ANIMATION_CONFIG.SEARCH_DURATION,
      easing: ANIMATION_CONFIG.SEARCH_EASING,
      fill: 'forwards'
    })
  }

  const createTabsShowAnimation = (): AnimationControls => {
    return useAnimate(tabsContainerRef, [
      {
        opacity: 0,
        transform: 'scaleX(0)',
        maxWidth: '0px'
      },
      // 添加中间关键帧，让显示更自然
      {
        opacity: 0.6,
        transform: 'scaleX(0.7)',
        maxWidth: '70%',
        offset: 0.4
      },
      {
        opacity: 1,
        transform: 'scaleX(1)',
        maxWidth: '100%'
      }
    ], {
      duration: ANIMATION_CONFIG.SEARCH_DURATION,
      easing: ANIMATION_CONFIG.SEARCH_EASING,
      fill: 'forwards'
    })
  }
  
  const resetSearchStyles = () => {
    nextTick(() => {
      if (searchContainerRef.value) {
        const el = searchContainerRef.value
        el.style.width = '0px'
        el.style.opacity = '0'
        el.style.transform = 'scaleX(0)'
        el.style.paddingLeft = '0px'
        el.style.paddingRight = '0px'
      }

      if (tabsContainerRef.value) {
        const el = tabsContainerRef.value
        el.style.opacity = '1'
        el.style.transform = 'scaleX(1)'
        el.style.maxWidth = '100%'
      }
    })
  }
  
  const initializeStyles = () => {
    resetSearchStyles()
  }
  
  // ==================== 状态设置方法 ====================
  
  const setSearchVisible = (visible: boolean) => {
    isSearchVisible.value = visible
  }
  
  const setSearchAnimating = (animating: boolean) => {
    isSearchAnimating.value = animating
  }
  
  const setOpeningSearch = (opening: boolean) => {
    isOpeningSearch.value = opening
  }
  
  // ==================== 返回接口 ====================
  
  return {
    // Overlay 动画状态
    currentAnimationState: readonly(overlayStateManager.state),
    isSlideInComplete: readonly(isSlideInComplete),
    
    // 搜索动画状态
    isSearchVisible: readonly(isSearchVisible),
    isSearchAnimating: readonly(isSearchAnimating),
    isOpeningSearch: readonly(isOpeningSearch),
    showSearchContainer: readonly(showSearchContainer),
    showTabsContainer: readonly(showTabsContainer),
    
    // Overlay 动画方法
    setAnimationState,
    canStartClosing,
    isAnimating,
    createSlideInAnimation,
    createSlideOutAnimation,
    resetOverlayState,
    
    // 搜索动画方法
    createSearchExpandAnimation,
    createSearchCollapseAnimation,
    createTabsHideAnimation,
    createTabsShowAnimation,
    resetSearchStyles,
    initializeStyles,
    setSearchVisible,
    setSearchAnimating,
    setOpeningSearch
  }
}
