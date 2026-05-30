import { defineStore } from 'pinia';
import { ref } from 'vue';
import { systemApi } from '@/api';

export const useSystemStore = defineStore('system', () => {
  // State
  const overlayVisible = ref(false);

  // Actions
  async function showOverlay() {
    try {
      await systemApi.showOverlay();
      overlayVisible.value = true;
    } catch (error) {
      console.error('显示 Overlay 失败:', error);
      throw error;
    }
  }

  async function hideOverlay() {
    try {
      await systemApi.hideOverlay();
      overlayVisible.value = false;
    } catch (error) {
      console.error('隐藏 Overlay 失败:', error);
      throw error;
    }
  }

  async function toggleOverlay() {
    try {
      await systemApi.toggleOverlay();
      overlayVisible.value = !overlayVisible.value;
    } catch (error) {
      console.error('切换 Overlay 失败:', error);
      throw error;
    }
  }

  return {
    // State
    overlayVisible,
    // Actions
    showOverlay,
    hideOverlay,
    toggleOverlay,
  };
});
