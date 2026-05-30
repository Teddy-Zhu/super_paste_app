import { invoke } from '@tauri-apps/api/core';
import type { ClipboardItem, ClipboardContent, QueryOptions } from './types';

export const clipboardApi = {
  /**
   * 获取剪贴板历史列表（分页）
   */
  async getHistory(options?: QueryOptions): Promise<ClipboardItem[]> {
    return invoke('get_clipboard_history', { options });
  },

  /**
   * 根据 ID 获取剪贴板条目详情（含完整内容）
   */
  async getItem(id: number): Promise<[ClipboardItem, ClipboardContent] | null> {
    return invoke('get_clipboard_item', { id });
  },

  /**
   * 搜索剪贴板历史（全文搜索）
   */
  async search(query: string, limit: number = 50): Promise<ClipboardItem[]> {
    return invoke('search_clipboard', { query, limit });
  },

  /**
   * 删除剪贴板条目
   */
  async deleteItem(id: number): Promise<void> {
    return invoke('delete_clipboard_item', { id });
  },

  /**
   * 清空所有非置顶/非收藏的剪贴板条目
   */
  async clearHistory(): Promise<number> {
    return invoke('clear_clipboard_history');
  },

  /**
   * 更新剪贴板条目的置顶/收藏状态
   */
  async updateFlags(
    id: number,
    isPinned?: boolean,
    isFavorite?: boolean
  ): Promise<void> {
    return invoke('update_clipboard_flags', {
      id,
      isPinned,
      isFavorite,
    });
  },

  /**
   * 获取剪贴板条目总数
   */
  async getCount(): Promise<number> {
    return invoke('get_clipboard_count');
  },
};
