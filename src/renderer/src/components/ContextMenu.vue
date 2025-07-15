<script setup lang="ts">
import { computed, ref } from 'vue'
import { useContextMenu } from '@renderer/composables/useContextMenu'
import IconChevronRight from '~icons/mdi/chevron-right'

const { contextMenuState, hideContextMenu } = useContextMenu()

// 二级菜单状态
const submenuState = ref({
  visible: false,
  x: 0,
  y: 0,
  items: [] as any[]
})

// 菜单悬停状态
const isMouseInMainMenu = ref(false)
const isMouseInSubmenu = ref(false)
let closeMenuTimer: number | null = null

// 计算菜单位置样式
const menuStyle = computed(() => {
  if (!contextMenuState.visible) return { display: 'none' }

  return {
    position: 'fixed' as const,
    zIndex: 9999,
    left: `${contextMenuState.x}px`,
    top: `${contextMenuState.y}px`,
  }
})

// 计算二级菜单位置样式
const submenuStyle = computed(() => {
  if (!submenuState.value.visible) return { display: 'none' }

  return {
    position: 'fixed' as const,
    zIndex: 10000,
    left: `${submenuState.value.x}px`,
    top: `${submenuState.value.y}px`,
  }
})

// 处理菜单项点击
const handleItemClick = (item: any) => {
  if (item.disabled) return

  if (item.submenu) {
    // 显示二级菜单
    showSubmenu(item)
  } else {
    // 执行操作并关闭菜单
    item.action()
    hideContextMenu()
    hideSubmenu()
  }
}

// 显示二级菜单
const showSubmenu = (item: any, event?: MouseEvent) => {
  const menuElement = document.querySelector('.context-menu')
  if (!menuElement) return

  const menuRect = menuElement.getBoundingClientRect()

  // 计算二级菜单位置，确保不超出屏幕边界
  let submenuX = menuRect.right - 2 // 减少间隙，避免鼠标移动时关闭菜单
  let submenuY = menuRect.top

  // 如果有鼠标事件，使用鼠标位置作为Y坐标参考
  if (event) {
    submenuY = event.clientY - 10
  }

  // 检查右边界
  const submenuWidth = 120 // 估算的二级菜单宽度
  if (submenuX + submenuWidth > window.innerWidth - 10) {
    submenuX = menuRect.left - submenuWidth + 2 // 显示在左侧，减少间隙
  }

  // 检查下边界
  const submenuHeight = item.submenu.length * 24 + 8 // 估算的二级菜单高度
  if (submenuY + submenuHeight > window.innerHeight - 10) {
    submenuY = window.innerHeight - submenuHeight - 10
  }

  submenuState.value = {
    visible: true,
    x: submenuX,
    y: submenuY,
    items: item.submenu
  }

  // 清除任何待执行的关闭定时器
  clearCloseMenuTimer()
}

// 清除关闭菜单定时器
const clearCloseMenuTimer = () => {
  if (closeMenuTimer) {
    clearTimeout(closeMenuTimer)
    closeMenuTimer = null
  }
}

// 隐藏二级菜单
const hideSubmenu = () => {
  submenuState.value.visible = false
  isMouseInSubmenu.value = false
  clearCloseMenuTimer()
}

// 处理二级菜单项点击
const handleSubmenuItemClick = (item: any) => {
  if (!item.disabled) {
    item.action()
    hideContextMenu()
    hideSubmenu()
  }
}

// 鼠标进入主菜单
const handleMainMenuMouseEnter = () => {
  isMouseInMainMenu.value = true
  clearCloseMenuTimer()
}

// 鼠标离开主菜单
const handleMainMenuMouseLeave = () => {
  isMouseInMainMenu.value = false
  // 延迟关闭，给用户时间移动到二级菜单
  closeMenuTimer = window.setTimeout(() => {
    // 只有当鼠标不在任何菜单中时才关闭菜单
    if (!isMouseInMainMenu.value && !isMouseInSubmenu.value) {
      hideSubmenu()
      // 如果没有二级菜单显示，也关闭主菜单
      if (!submenuState.value.visible) {
        hideContextMenu()
      }
    }
  }, 150)
}

// 鼠标进入二级菜单
const handleSubmenuMouseEnter = () => {
  isMouseInSubmenu.value = true
  clearCloseMenuTimer()
}

// 鼠标离开二级菜单
const handleSubmenuMouseLeave = () => {
  isMouseInSubmenu.value = false
  // 延迟关闭，给用户时间移动回主菜单
  closeMenuTimer = window.setTimeout(() => {
    // 只有当鼠标不在任何菜单中时才关闭
    if (!isMouseInMainMenu.value && !isMouseInSubmenu.value) {
      hideSubmenu()
      hideContextMenu() // 同时关闭主菜单
    }
  }, 150)
}

// 鼠标进入菜单项时的处理
const handleMenuItemMouseEnter = (item: any, event: MouseEvent) => {
  clearCloseMenuTimer()

  if (item.submenu) {
    showSubmenu(item, event)
  } else {
    hideSubmenu()
  }
}


</script>

<template>
  <!-- 主菜单 -->
  <div
    v-if="contextMenuState.visible"
    class="context-menu"
    :style="menuStyle"
    @mouseenter="handleMainMenuMouseEnter"
    @mouseleave="handleMainMenuMouseLeave"
  >
    <div class="menu-item-list">
      <template v-for="(item, index) in contextMenuState.items" :key="index">
        <!-- 分隔符 -->
        <div v-if="item.label === '---'" class="menu-separator"></div>
        <!-- 普通菜单项 -->
        <div
          v-else
          class="menu-item"
          :class="{ disabled: item.disabled, 'has-submenu': item.submenu }"
          @click="handleItemClick(item)"
          @mouseenter="(event) => handleMenuItemMouseEnter(item, event)"
        >
          <span
            v-if="item.icon"
            class="menu-item-icon"
            :style="{ color: item.iconColor || 'inherit' }"
          >
            <component v-if="typeof item.icon !== 'string'" :is="item.icon" />
            <span v-else>{{ item.icon }}</span>
          </span>
          <span class="menu-item-label">{{ item.label }}</span>
          <span v-if="item.submenu" class="submenu-arrow">
            <IconChevronRight />
          </span>
        </div>
      </template>
    </div>
  </div>

  <!-- 二级菜单 -->
  <div
    v-if="submenuState.visible"
    class="context-menu submenu"
    :style="submenuStyle"
    @mouseenter="handleSubmenuMouseEnter"
    @mouseleave="handleSubmenuMouseLeave"
  >
    <div class="menu-item-list">
      <template v-for="(item, index) in submenuState.items" :key="index">
        <!-- 分隔符 -->
        <div v-if="item.label === '---'" class="menu-separator"></div>
        <!-- 二级菜单项 -->
        <div
          v-else
          class="menu-item"
          :class="{ disabled: item.disabled }"
          @click="handleSubmenuItemClick(item)"
        >
          <span
            v-if="item.icon"
            class="menu-item-icon"
            :style="{ color: item.iconColor || 'inherit' }"
          >
            <component v-if="typeof item.icon !== 'string'" :is="item.icon" />
            <span v-else>{{ item.icon }}</span>
          </span>
          <span class="menu-item-label">{{ item.label }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.context-menu {
  background: rgba(255, 255, 255, 0.95);
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 6px;
  box-shadow:
    0 2px 12px rgba(0, 0, 0, 0.12),
    0 1px 4px rgba(0, 0, 0, 0.08);
  backdrop-filter: blur(10px);
  min-width: 120px;
  padding: 2px 0;
  font-size: 12px;
  user-select: none;
}

.submenu {
  min-width: 100px;
}

.menu-item-list {
  display: flex;
  flex-direction: column;
}

.menu-item {
  display: flex;
  align-items: center;
  padding: 6px 12px;
  cursor: pointer;
  transition: background-color 0.15s ease;
  color: #333;
  position: relative;
}

.menu-item:hover:not(.disabled) {
  background-color: rgba(0, 123, 255, 0.08);
  color: #007bff;
}

.menu-item.disabled {
  color: #999;
  cursor: not-allowed;
}

.menu-item.has-submenu {
  padding-right: 24px;
}

.menu-item-icon {
  margin-right: 6px;
  font-size: 12px;
  width: 12px;
  height: 12px;
  text-align: center;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.menu-item-label {
  flex: 1;
  white-space: nowrap;
  font-size: 12px;
}

.submenu-arrow {
  position: absolute;
  right: 8px;
  font-size: 10px;
  color: #666;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
}

.menu-separator {
  height: 1px;
  background-color: rgba(0, 0, 0, 0.08);
  margin: 2px 6px;
}
</style>
