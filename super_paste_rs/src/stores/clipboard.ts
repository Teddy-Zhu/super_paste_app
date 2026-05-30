import { defineStore } from 'pinia';
import { ref, computed } from 'vue';
import { clipboardApi, type ClipboardItem, type QueryOptions } from '@/api';

export const useClipboardStore = defineStore('clipboard', () => {
  // State
  const items = ref<ClipboardItem[]>([]);
  const currentItem = ref<ClipboardItem | null>(null);
  const loading = ref(false);
  const searchQuery = ref('');
  const totalCount = ref(0);

  // Computed
  const pinnedItems = computed(() => items.value.filter((item) => item.is_pinned));
  const favoriteItems = computed(() => items.value.filter((item) => item.is_favorite));

  // Actions
  async function fetchHistory(options?: QueryOptions) {
    loading.value = true;
    try {
      items.value = await clipboardApi.getHistory(options);
      totalCount.value = await clipboardApi.getCount();
    } catch (error) {
      console.error('获取剪贴板历史失败:', error);
      throw error;
    } finally {
      loading.value = false;
    }
  }

  async function search(query: string) {
    if (!query.trim()) {
      await fetchHistory();
      return;
    }
    loading.value = true;
    try {
      searchQuery.value = query;
      items.value = await clipboardApi.search(query);
    } catch (error) {
      console.error('搜索失败:', error);
      throw error;
    } finally {
      loading.value = false;
    }
  }

  async function deleteItem(id: number) {
    try {
      await clipboardApi.deleteItem(id);
      items.value = items.value.filter((item) => item.id !== id);
      totalCount.value--;
    } catch (error) {
      console.error('删除失败:', error);
      throw error;
    }
  }

  async function clearHistory() {
    try {
      const deleted = await clipboardApi.clearHistory();
      await fetchHistory();
      return deleted;
    } catch (error) {
      console.error('清空历史失败:', error);
      throw error;
    }
  }

  async function togglePin(id: number) {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;

    try {
      await clipboardApi.updateFlags(id, !item.is_pinned, undefined);
      item.is_pinned = !item.is_pinned;
    } catch (error) {
      console.error('切换置顶失败:', error);
      throw error;
    }
  }

  async function toggleFavorite(id: number) {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;

    try {
      await clipboardApi.updateFlags(id, undefined, !item.is_favorite);
      item.is_favorite = !item.is_favorite;
    } catch (error) {
      console.error('切换收藏失败:', error);
      throw error;
    }
  }

  return {
    // State
    items,
    currentItem,
    loading,
    searchQuery,
    totalCount,
    // Computed
    pinnedItems,
    favoriteItems,
    // Actions
    fetchHistory,
    search,
    deleteItem,
    clearHistory,
    togglePin,
    toggleFavorite,
  };
});
