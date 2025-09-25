/**
 * 搜索框动画管理 Composable
 * 管理搜索框的展开/收缩动画和状态
 */

import { ref, computed, nextTick, readonly, type Ref } from 'vue'
import { useAnimate } from '@vueuse/core'
import type {
  SearchAnimationManager,
  SearchAnimationConfig,
  AnimationControls
} from '@renderer/types/overlay'

// 搜索动画配置
export const SEARCH_ANIMATION_CONFIG: SearchAnimationConfig = {
  DURATION: 300,
  EASING: 'cubic-bezier(0.4, 0, 0.2, 1)',
  SEARCH_WIDTH: '600px',
  COLLAPSE_WIDTH: '400px'
} as const

export function useSearchAnimation(
  searchContainerRef: Ref<HTMLElement | undefined>,
  tabsContainerRef: Ref<HTMLElement | undefined>
): SearchAnimationManager {
  // 搜索状态
  const isSearchVisible = ref(false)
  const isSearchAnimating = ref(false)
  const isOpeningSearch = ref(false)

  // 计算属性：控制元素可见性，在动画期间两个元素都可见
  const showSearchContainer = computed(() => {
    return isSearchVisible.value || isSearchAnimating.value
  })

  const showTabsContainer = computed(() => {
    return !isSearchVisible.value || isSearchAnimating.value
  })

  /**
   * 搜索框展开动画
   */
  function createSearchExpandAnimation(): AnimationControls {
    return useAnimate(
      searchContainerRef,
      [
        {
          width: '0px',
          opacity: 0,
          transform: 'scaleX(0)',
          paddingLeft: '0px',
          paddingRight: '0px'
        },
        {
          width: SEARCH_ANIMATION_CONFIG.SEARCH_WIDTH,
          opacity: 1,
          transform: 'scaleX(1)',
          paddingLeft: '8px',
          paddingRight: '8px'
        }
      ],
      {
        duration: SEARCH_ANIMATION_CONFIG.DURATION,
        easing: SEARCH_ANIMATION_CONFIG.EASING,
        fill: 'forwards'
      }
    )
  }

  /**
   * 搜索框缩回动画
   */
  function createSearchCollapseAnimation(): AnimationControls {
    return useAnimate(
      searchContainerRef,
      [
        {
          width: SEARCH_ANIMATION_CONFIG.COLLAPSE_WIDTH,
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
      ],
      {
        duration: SEARCH_ANIMATION_CONFIG.DURATION,
        easing: SEARCH_ANIMATION_CONFIG.EASING,
        fill: 'forwards'
      }
    )
  }

  /**
   * 标签组隐藏动画
   */
  function createTabsHideAnimation(): AnimationControls {
    return useAnimate(
      tabsContainerRef,
      [
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
      ],
      {
        duration: SEARCH_ANIMATION_CONFIG.DURATION,
        easing: SEARCH_ANIMATION_CONFIG.EASING,
        fill: 'forwards'
      }
    )
  }

  /**
   * 标签组显示动画
   */
  function createTabsShowAnimation(): AnimationControls {
    return useAnimate(
      tabsContainerRef,
      [
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
      ],
      {
        duration: SEARCH_ANIMATION_CONFIG.DURATION,
        easing: SEARCH_ANIMATION_CONFIG.EASING,
        fill: 'forwards'
      }
    )
  }

  /**
   * 重置搜索框样式到初始状态
   */
  function resetSearchStyles() {
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

  /**
   * 初始化搜索框和标签组的状态
   */
  function initializeStyles() {
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
  }

  return {
    // 状态
    isSearchVisible: readonly(isSearchVisible),
    isSearchAnimating: readonly(isSearchAnimating),
    isOpeningSearch: readonly(isOpeningSearch),
    showSearchContainer,
    showTabsContainer,

    // 内部状态修改方法（仅供内部使用）
    _setSearchVisible: (value: boolean) => {
      isSearchVisible.value = value
    },
    _setSearchAnimating: (value: boolean) => {
      isSearchAnimating.value = value
    },
    _setOpeningSearch: (value: boolean) => {
      isOpeningSearch.value = value
    },

    // 动画方法
    createSearchExpandAnimation,
    createSearchCollapseAnimation,
    createTabsHideAnimation,
    createTabsShowAnimation,
    resetSearchStyles,
    initializeStyles
  }
}
