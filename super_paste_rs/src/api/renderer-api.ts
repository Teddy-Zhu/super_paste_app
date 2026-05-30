/**
 * 渲染进程 API 封装 - Tauri 版本
 * 兼容原 Electron 项目的 API 接口
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  ClipboardItem,
  ClipboardContent,
  QueryOptions,
  AppConfig,
} from './types';

// ==================== 类型定义（兼容原项目）====================

export interface ClipboardHistoryQueryOptions {
  limit?: number;
  offset?: number;
  tagIds?: string[];
  searchText?: string;
  orderBy?: string;
  orderDirection?: 'ASC' | 'DESC';
}

export interface ClipboardHistoryResult {
  items: ClipboardItem[];
  apps: Record<string, AppInfo>;
}

export interface AppInfo {
  id: string;
  name: string;
  icon?: string;
}

export interface TagGroup {
  id: string;
  name: string;
  color: string;
  count: number;
}

export interface ClipboardIpcResponse<T = any> {
  success: boolean;
  data?: T;
  apps?: Record<string, AppInfo>;
  error?: string;
  message?: string;
}

// ==================== Store 操作 ====================

export const store = {
  async get<T = any>(key: string, defaultValue?: T): Promise<T> {
    try {
      const value = await invoke<T | null>('get_config_value', { key });
      return value ?? defaultValue!;
    } catch (error) {
      console.error('Store get error:', error);
      return defaultValue!;
    }
  },

  async set<T = any>(key: string, value: T): Promise<void> {
    await invoke('set_config_value', { key, value });
  },
};

// ==================== Clipboard 操作 ====================

export const clipboard = {
  async getHistory(
    options?: ClipboardHistoryQueryOptions | number
  ): Promise<ClipboardHistoryResult> {
    const queryOptions: QueryOptions =
      typeof options === 'number'
        ? { limit: options }
        : {
            limit: options?.limit,
            offset: options?.offset,
            // tag_ids: options?.tagIds,
            // search_text: options?.searchText,
          };

    try {
      const items = await invoke<ClipboardItem[]>('get_clipboard_history', {
        options: queryOptions,
      });
      return {
        items: items || [],
        apps: {}, // TODO: 实现应用信息获取
      };
    } catch (error) {
      console.error('Get clipboard history error:', error);
      return { items: [], apps: {} };
    }
  },

  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardHistoryResult> {
    return this.getHistory(options);
  },

  async restore(itemId: string): Promise<boolean> {
    try {
      await invoke('restore_clipboard_item', { itemId });
      return true;
    } catch (error) {
      console.error('Restore clipboard error:', error);
      return false;
    }
  },

  async restoreAsText(itemId: string): Promise<boolean> {
    try {
      await invoke('restore_clipboard_as_text', { itemId });
      return true;
    } catch (error) {
      console.error('Restore as text error:', error);
      return false;
    }
  },

  async restoreOcrText(itemId: string): Promise<boolean> {
    try {
      await invoke('restore_clipboard_ocr_text', { itemId });
      return true;
    } catch (error) {
      console.error('Restore OCR text error:', error);
      return false;
    }
  },

  async parseOcr(itemId: string): Promise<boolean> {
    try {
      await invoke('parse_clipboard_ocr', { itemId });
      return true;
    } catch (error) {
      console.error('Parse OCR error:', error);
      return false;
    }
  },

  async parseQRCode(itemId: string): Promise<boolean> {
    try {
      await invoke('parse_clipboard_qrcode', { itemId });
      return true;
    } catch (error) {
      console.error('Parse QR code error:', error);
      return false;
    }
  },

  async generateQRCode(itemId: string): Promise<boolean> {
    try {
      await invoke('generate_clipboard_qrcode', { itemId });
      return true;
    } catch (error) {
      console.error('Generate QR code error:', error);
      return false;
    }
  },

  async deleteItem(itemId: string): Promise<boolean> {
    try {
      const id = parseInt(itemId, 10);
      await invoke('delete_clipboard_item', { id });
      return true;
    } catch (error) {
      console.error('Delete clipboard item error:', error);
      return false;
    }
  },

  async clearHistory(): Promise<boolean> {
    try {
      await invoke('clear_clipboard_history');
      return true;
    } catch (error) {
      console.error('Clear clipboard history error:', error);
      return false;
    }
  },

  async togglePin(itemId: string): Promise<boolean> {
    try {
      const id = parseInt(itemId, 10);
      // TODO: 需要先获取当前状态，然后切换
      await invoke('update_clipboard_flags', { id, isPinned: true });
      return true;
    } catch (error) {
      console.error('Toggle pin error:', error);
      return false;
    }
  },

  async toggleFavorite(itemId: string): Promise<boolean> {
    try {
      const id = parseInt(itemId, 10);
      // TODO: 需要先获取当前状态，然后切换
      await invoke('update_clipboard_flags', { id, isFavorite: true });
      return true;
    } catch (error) {
      console.error('Toggle favorite error:', error);
      return false;
    }
  },
};

// ==================== Tags 操作 ====================

export const tags = {
  async getAll(): Promise<TagGroup[]> {
    try {
      // TODO: 实现标签获取
      return [];
    } catch (error) {
      console.error('Get all tags error:', error);
      return [];
    }
  },

  async create(name: string, color: string, description?: string): Promise<string> {
    try {
      // TODO: 实现标签创建
      return '';
    } catch (error) {
      console.error('Create tag error:', error);
      throw error;
    }
  },

  async update(
    tagId: string,
    updates: { name?: string; color?: string; description?: string }
  ): Promise<boolean> {
    try {
      // TODO: 实现标签更新
      return true;
    } catch (error) {
      console.error('Update tag error:', error);
      return false;
    }
  },

  async delete(tagId: string): Promise<boolean> {
    try {
      // TODO: 实现标签删除
      return true;
    } catch (error) {
      console.error('Delete tag error:', error);
      return false;
    }
  },

  async bindToRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      // TODO: 实现标签绑定
      return true;
    } catch (error) {
      console.error('Bind tag error:', error);
      return false;
    }
  },

  async unbindFromRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      // TODO: 实现标签解绑
      return true;
    } catch (error) {
      console.error('Unbind tag error:', error);
      return false;
    }
  },
};

// ==================== App 操作 ====================

export const app = {
  async restart(): Promise<boolean> {
    try {
      await invoke('restart_app');
      return true;
    } catch (error) {
      console.error('Restart app error:', error);
      return false;
    }
  },

  async minimizeWindow(): Promise<boolean> {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().minimize();
      return true;
    } catch (error) {
      console.error('Minimize window error:', error);
      return false;
    }
  },

  async closeWindow(): Promise<boolean> {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().hide();
      return true;
    } catch (error) {
      console.error('Close window error:', error);
      return false;
    }
  },
};

// ==================== IPC 事件监听（兼容层）====================

export function on(channel: string, callback: (...args: any[]) => void): () => void {
  // Tauri 使用事件系统
  import('@tauri-apps/api/event').then(({ listen }) => {
    listen(channel, (event) => {
      callback(event, event.payload);
    });
  });
  return () => {}; // TODO: 返回取消监听函数
}

export function onSimple(channel: string, callback: (...args: any[]) => void): () => void {
  import('@tauri-apps/api/event').then(({ listen }) => {
    listen(channel, (event) => {
      callback(event.payload);
    });
  });
  return () => {};
}

export function once(channel: string, callback: (...args: any[]) => void): () => void {
  import('@tauri-apps/api/event').then(({ once }) => {
    once(channel, (event) => {
      callback(event, event.payload);
    });
  });
  return () => {};
}

export function onceSimple(channel: string, callback: (...args: any[]) => void): () => void {
  import('@tauri-apps/api/event').then(({ once }) => {
    once(channel, (event) => {
      callback(event.payload);
    });
  });
  return () => {};
}

export function send(channel: string, ...args: any[]): void {
  import('@tauri-apps/api/event').then(({ emit }) => {
    emit(channel, args);
  });
}

export async function invokeIpc<T = any>(channel: string, ...args: any[]): Promise<T> {
  return invoke(channel, args[0]);
}

export function removeListener(channel: string, callback: (...args: any[]) => void): void {
  // TODO: 实现移除监听器
}

export function removeAllListeners(channel: string): void {
  // TODO: 实现移除所有监听器
}

// 导出兼容的 API
export default {
  store,
  clipboard,
  tags,
  app,
  on,
  onSimple,
  once,
  onceSimple,
  send,
  invoke: invokeIpc,
  removeListener,
  removeAllListeners,
};
