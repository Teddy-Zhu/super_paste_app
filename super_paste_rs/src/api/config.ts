import { invoke } from '@tauri-apps/api/core';
import type { AppConfig } from './types';

export const configApi = {
  /**
   * 获取完整配置
   */
  async getConfig(): Promise<AppConfig> {
    return invoke('get_config');
  },

  /**
   * 更新单个配置项
   */
  async setValue(key: string, value: string): Promise<void> {
    return invoke('set_config_value', { key, value });
  },

  /**
   * 获取单个配置项
   */
  async getValue(key: string): Promise<string | null> {
    return invoke('get_config_value', { key });
  },
};
