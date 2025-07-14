<script setup lang="ts">
import { computed } from 'vue'
import { useContextMenu } from '@renderer/composables/useContextMenu'

const { contextMenuState, hideContextMenu } = useContextMenu()

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

// 处理菜单项点击
const handleItemClick = (item: any) => {
  if (!item.disabled) {
    item.action()
    hideContextMenu()
  }
}
</script>

<template>
  <div
    v-if="contextMenuState.visible"
    class="context-menu"
    :style="menuStyle"
  >
    <div class="menu-item-list">
      <div
        v-for="(item, index) in contextMenuState.items"
        :key="index"
        class="menu-item"
        :class="{ disabled: item.disabled }"
        @click="handleItemClick(item)"
      >
        <span v-if="item.icon" class="menu-item-icon">{{ item.icon }}</span>
        <span class="menu-item-label">{{ item.label }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.context-menu {
  background: rgba(255, 255, 255, 0.95);
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  box-shadow: 
    0 4px 20px rgba(0, 0, 0, 0.15),
    0 2px 8px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(10px);
  min-width: 160px;
  padding: 4px 0;
  font-size: 14px;
  user-select: none;
}

.menu-item-list {
  display: flex;
  flex-direction: column;
}

.menu-item {
  display: flex;
  align-items: center;
  padding: 8px 16px;
  cursor: pointer;
  transition: background-color 0.2s ease;
  color: #333;
}

.menu-item:hover:not(.disabled) {
  background-color: rgba(0, 123, 255, 0.1);
  color: #007bff;
}

.menu-item.disabled {
  color: #999;
  cursor: not-allowed;
}

.menu-item-icon {
  margin-right: 8px;
  font-size: 16px;
  width: 16px;
  text-align: center;
}

.menu-item-label {
  flex: 1;
  white-space: nowrap;
}
</style>
