<script setup lang="ts">
import { computed, onUnmounted, ref, type CSSProperties } from 'vue'
import { useContextMenu, type ContextMenuItem } from '@renderer/composables/useContextMenu'
import IconChevronRight from '~icons/mdi/chevron-right'

const { contextMenuState, hideContextMenu } = useContextMenu()

const SUBMENU_WIDTH = 120
const MENU_ITEM_HEIGHT = 24
const MENU_PADDING = 10
const VIEWPORT_MARGIN = 10
const MENU_CLOSE_DELAY = 150

const submenuState = ref<{
  visible: boolean
  x: number
  y: number
  items: ContextMenuItem[]
}>({
  visible: false,
  x: 0,
  y: 0,
  items: []
})

const isMouseInMainMenu = ref(false)
const isMouseInSubmenu = ref(false)
let closeMenuTimer: number | null = null

const getMenuStyle = (visible: boolean, x: number, y: number, zIndex: number): CSSProperties => {
  if (!visible) {
    return { display: 'none' }
  }

  return {
    position: 'fixed' as const,
    zIndex,
    left: `${x}px`,
    top: `${y}px`
  }
}

const menuStyle = computed(() =>
  getMenuStyle(contextMenuState.visible, contextMenuState.x, contextMenuState.y, 9999)
)

const submenuStyle = computed(() =>
  getMenuStyle(submenuState.value.visible, submenuState.value.x, submenuState.value.y, 10000)
)

const clearCloseMenuTimer = (): void => {
  if (closeMenuTimer !== null) {
    clearTimeout(closeMenuTimer)
    closeMenuTimer = null
  }
}

const scheduleMenuClose = (callback: () => void): void => {
  clearCloseMenuTimer()
  closeMenuTimer = window.setTimeout(() => {
    if (!isMouseInMainMenu.value && !isMouseInSubmenu.value) {
      callback()
    }
  }, MENU_CLOSE_DELAY)
}

const hideSubmenu = (): void => {
  submenuState.value = {
    ...submenuState.value,
    visible: false,
    items: []
  }
  isMouseInSubmenu.value = false
  clearCloseMenuTimer()
}

const closeAllMenus = (): void => {
  hideSubmenu()
  hideContextMenu()
}

const handleItemClick = (item: ContextMenuItem): void => {
  if (item.disabled) return

  if (item.submenu?.length) {
    showSubmenu(item)
    return
  }

  item.action()
  closeAllMenus()
}

const showSubmenu = (item: ContextMenuItem, event?: MouseEvent): void => {
  const submenuItems = item.submenu ?? []
  if (submenuItems.length === 0) {
    hideSubmenu()
    return
  }

  const menuElement = document.querySelector('.context-menu:not(.submenu)')
  if (!menuElement) return

  const menuRect = menuElement.getBoundingClientRect()

  let submenuX = menuRect.right - 2
  let submenuY = event ? event.clientY - 10 : menuRect.top

  if (submenuX + SUBMENU_WIDTH > window.innerWidth - VIEWPORT_MARGIN) {
    submenuX = menuRect.left - SUBMENU_WIDTH + 2
  }

  const submenuHeight = submenuItems.length * MENU_ITEM_HEIGHT + MENU_PADDING
  if (submenuY + submenuHeight > window.innerHeight - VIEWPORT_MARGIN) {
    submenuY = window.innerHeight - submenuHeight - VIEWPORT_MARGIN
  }

  submenuState.value = {
    visible: true,
    x: submenuX,
    y: submenuY,
    items: submenuItems
  }

  clearCloseMenuTimer()
}

const handleSubmenuItemClick = (item: ContextMenuItem): void => {
  if (item.disabled) return
  item.action()
  closeAllMenus()
}

const handleMainMenuMouseEnter = (): void => {
  isMouseInMainMenu.value = true
  clearCloseMenuTimer()
}

const handleMainMenuMouseLeave = (): void => {
  isMouseInMainMenu.value = false
  scheduleMenuClose(closeAllMenus)
}

const handleSubmenuMouseEnter = (): void => {
  isMouseInSubmenu.value = true
  clearCloseMenuTimer()
}

const handleSubmenuMouseLeave = (): void => {
  isMouseInSubmenu.value = false
  scheduleMenuClose(closeAllMenus)
}

const handleMenuItemMouseEnter = (item: ContextMenuItem, event: Event): void => {
  clearCloseMenuTimer()

  if (item.submenu?.length) {
    showSubmenu(item, event as MouseEvent)
  } else {
    hideSubmenu()
  }
}

onUnmounted(clearCloseMenuTimer)
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
          :class="{ disabled: item.disabled, 'has-submenu': item.submenu?.length }"
          @click="handleItemClick(item)"
          @mouseenter="handleMenuItemMouseEnter(item, $event)"
        >
          <span
            v-if="item.icon"
            class="menu-item-icon"
            :style="{ color: item.iconColor || 'inherit' }"
          >
            <component :is="item.icon" v-if="typeof item.icon !== 'string'" />
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
            <component :is="item.icon" v-if="typeof item.icon !== 'string'" />
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
  animation: menu-appear var(--duration-fast) var(--ease-out);
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-strong);
  border-radius: 6px;
  box-shadow:
    0 2px 12px rgba(0, 0, 0, 0.12),
    0 1px 4px rgba(0, 0, 0, 0.08);
  width: 160px;
  padding: 4px;
  font-size: 12px;
  user-select: none;
}

.submenu {
  width: 120px;
}

.menu-item-list {
  display: flex;
  flex-direction: column;
}

.menu-item {
  display: flex;
  align-items: center;
  min-height: 24px;
  padding: 3px 8px;
  border-radius: var(--radius-xs);
  cursor: pointer;
  transition: background-color var(--duration-fast);
  color: var(--color-text-primary);
  position: relative;
}

.menu-item:hover:not(.disabled) {
  background-color: var(--color-selection);
  color: var(--color-primary);
}

.menu-item:active:not(.disabled) {
  background: var(--color-selection-hover);
}

.menu-item.disabled {
  color: var(--color-text-tertiary);
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
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
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
@keyframes menu-appear {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}
</style>
