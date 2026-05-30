import { defineStore } from 'pinia';
import { ref } from 'vue';
import { configApi, type AppConfig } from '@/api';

export const useConfigStore = defineStore('config', () => {
  // State
  const config = ref<AppConfig>({
    max_history_items: 1000,
    auto_paste_delay_ms: 100,
    enable_ocr: false,
    enable_sync: false,
    sync_server_url: '',
    sync_username: '',
    sync_password: '',
    global_shortcut: 'CmdOrCtrl+Shift+V',
    overlay_position: 'center',
    theme: 'auto',
    language: 'zh-CN',
  });

  const loading = ref(false);

  // Actions
  async function loadConfig() {
    loading.value = true;
    try {
      config.value = await configApi.getConfig();
    } catch (error) {
      console.error('加载配置失败:', error);
      throw error;
    } finally {
      loading.value = false;
    }
  }

  async function updateValue(key: string, value: string) {
    try {
      await configApi.setValue(key, value);
      // 更新本地状态
      (config.value as any)[key] = value;
    } catch (error) {
      console.error('更新配置失败:', error);
      throw error;
    }
  }

  async function updateConfig(partial: Partial<AppConfig>) {
    try {
      for (const [key, value] of Object.entries(partial)) {
        await configApi.setValue(key, String(value));
      }
      // 重新加载完整配置
      await loadConfig();
    } catch (error) {
      console.error('批量更新配置失败:', error);
      throw error;
    }
  }

  return {
    // State
    config,
    loading,
    // Actions
    loadConfig,
    updateValue,
    updateConfig,
  };
});
