<template>
  <div class="overlay-container" :class="{ 'side-layout': isSideLayout }">
    <div class="search-container">
      <input
        v-model="searchQuery"
        type="text"
        :placeholder="t('common.search')"
        class="search-input"
        @input="handleSearch"
        @keydown.down.prevent="handleArrowDown"
        @keydown.up.prevent="handleArrowUp"
        @keydown.enter.prevent="handleEnter"
        @keydown.esc="handleEscape"
      />
    </div>

    <div class="clipboard-list" ref="listRef">
      <div
        v-for="(item, index) in filteredItems"
        :key="item.id"
        class="clipboard-item"
        :class="{ selected: index === selectedIndex }"
        @click="handleItemClick(item)"
        @mouseenter="selectedIndex = index"
      >
        <div class="item-header">
          <span class="item-type">{{ getTypeLabel(item.primary_type) }}</span>
          <span class="item-time">{{ formatTime(item.last_used) }}</span>
        </div>
        <div class="item-content">
          {{ item.preview_text || getTypeLabel(item.primary_type) }}
        </div>
        <div v-if="item.is_pinned || item.is_favorite" class="item-badges">
          <span v-if="item.is_pinned" class="badge">📌</span>
          <span v-if="item.is_favorite" class="badge">⭐</span>
        </div>
      </div>

      <div v-if="filteredItems.length === 0" class="empty-state">
        {{ t('clipboard.empty') }}
      </div>
    </div>

    <div class="footer">
      <span>{{ filteredItems.length }} / {{ clipboardStore.totalCount }}</span>
      <button @click="openSettings" class="btn-settings">⚙️</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, nextTick } from 'vue';
import { useI18n } from 'vue-i18n';
import { useClipboardStore, useConfigStore } from '@/stores';
import { useDebounce } from '@/composables/useUtils';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';

const { t } = useI18n();
const clipboardStore = useClipboardStore();
const configStore = useConfigStore();

const searchQuery = ref('');
const selectedIndex = ref(0);
const listRef = ref<HTMLElement>();

const isSideLayout = computed(() => {
  const pos = configStore.config.overlay_position;
  return pos === 'left' || pos === 'right';
});

const filteredItems = computed(() => {
  if (!searchQuery.value.trim()) {
    return clipboardStore.items;
  }
  const query = searchQuery.value.toLowerCase();
  return clipboardStore.items.filter(item =>
    item.preview_text?.toLowerCase().includes(query)
  );
});

const handleSearch = useDebounce(async () => {
  if (searchQuery.value.trim()) {
    await clipboardStore.search(searchQuery.value);
  } else {
    await clipboardStore.fetchHistory();
  }
  selectedIndex.value = 0;
}, 300);

const handleArrowDown = () => {
  if (selectedIndex.value < filteredItems.value.length - 1) {
    selectedIndex.value++;
    scrollToSelected();
  }
};

const handleArrowUp = () => {
  if (selectedIndex.value > 0) {
    selectedIndex.value--;
    scrollToSelected();
  }
};

const handleEnter = async () => {
  const item = filteredItems.value[selectedIndex.value];
  if (item) {
    await handleItemClick(item);
  }
};

const handleEscape = async () => {
  await invoke('hide_overlay_window');
};

const handleItemClick = async (item: any) => {
  if (item.preview_text) {
    await navigator.clipboard.writeText(item.preview_text);
  }
  await invoke('hide_overlay_window');
};

const scrollToSelected = () => {
  nextTick(() => {
    const list = listRef.value;
    if (!list) return;
    const items = list.querySelectorAll('.clipboard-item');
    const selected = items[selectedIndex.value];
    if (selected) {
      selected.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
  });
};

const openSettings = async () => {
  await invoke('hide_overlay_window');
  const mainWindow = await import('@tauri-apps/api/window').then(m => m.Window.getByLabel('main'));
  if (mainWindow) {
    await mainWindow.show();
    await mainWindow.setFocus();
  }
};

const getTypeLabel = (type: string): string => {
  const typeMap: Record<string, string> = {
    text: t('clipboard.types.text'),
    image: t('clipboard.types.image'),
    file: t('clipboard.types.file'),
    html: t('clipboard.types.html'),
    rtf: t('clipboard.types.rtf'),
  };
  return typeMap[type] || type;
};

const formatTime = (timestamp: string): string => {
  const date = new Date(timestamp);
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(diff / 3600000);
  const days = Math.floor(diff / 86400000);

  if (minutes < 1) return '刚刚';
  if (minutes < 60) return `${minutes}分钟前`;
  if (hours < 24) return `${hours}小时前`;
  if (days < 7) return `${days}天前`;
  return date.toLocaleDateString();
};

onMounted(async () => {
  await clipboardStore.fetchHistory({ limit: 50 });

  // 监听窗口显示事件
  const currentWindow = getCurrentWindow();
  currentWindow.listen('tauri://focus', async () => {
    await clipboardStore.fetchHistory({ limit: 50 });
    selectedIndex.value = 0;
    searchQuery.value = '';
  });
});
</script>

<style scoped>
.overlay-container {
  display: flex;
  flex-direction: column;
  height: 100vh;
  background: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(10px);
  border-radius: 12px;
  overflow: hidden;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
}

.side-layout {
  flex-direction: row;
}

.search-container {
  padding: 12px;
  border-bottom: 1px solid #e0e0e0;
}

.search-input {
  width: 100%;
  padding: 10px 16px;
  border: 1px solid #d0d0d0;
  border-radius: 8px;
  font-size: 14px;
  outline: none;
  transition: border-color 0.2s;
}

.search-input:focus {
  border-color: #007bff;
}

.clipboard-list {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

.clipboard-item {
  padding: 12px;
  margin-bottom: 4px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.2s;
  position: relative;
}

.clipboard-item:hover,
.clipboard-item.selected {
  background: #f0f0f0;
}

.item-header {
  display: flex;
  justify-content: space-between;
  margin-bottom: 6px;
  font-size: 12px;
}

.item-type {
  color: #666;
  font-weight: 500;
}

.item-time {
  color: #999;
}

.item-content {
  font-size: 14px;
  color: #333;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  line-height: 1.4;
}

.item-badges {
  position: absolute;
  top: 8px;
  right: 8px;
  display: flex;
  gap: 4px;
}

.badge {
  font-size: 12px;
}

.empty-state {
  text-align: center;
  padding: 40px 20px;
  color: #999;
}

.footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 12px;
  border-top: 1px solid #e0e0e0;
  font-size: 12px;
  color: #666;
}

.btn-settings {
  padding: 4px 8px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 16px;
  opacity: 0.6;
  transition: opacity 0.2s;
}

.btn-settings:hover {
  opacity: 1;
}
</style>
