/**
 * 渲染进程 API 封装 - Tauri 版本
 */

import { invoke } from './ipc'
import type {
  StoreOperations,
  ClipboardOperations,
  ClipboardItem,
  ClipboardStatus,
  ClipboardManagerConfig,
  ClipboardIpcResponse,
  ClipboardHistoryQueryOptions,
  ClipboardHistoryResult,
  TagGroup,
  TagStats,
  AppInfo,
  AppCacheStats,
  AppOperations
} from './types'

export const store: StoreOperations = {
  async get<T = any>(key: string, defaultValue?: T): Promise<T> {
    const result = await invoke<{ success: boolean; data?: T; error?: string }>('store-get', {
      key,
      defaultValue
    })
    if (result.success) return result.data ?? defaultValue!
    throw new Error(result.error || '获取存储值失败')
  },

  async set<T = any>(key: string, value: T): Promise<void> {
    const result = await invoke<{ success: boolean; error?: string }>('store-set', { key, value })
    if (!result.success) throw new Error(result.error || '设置存储值失败')
  }
}

export const clipboard: ClipboardOperations = {
  async getHistory(options?: ClipboardHistoryQueryOptions | number): Promise<ClipboardHistoryResult> {
    const queryOptions = typeof options === 'number' ? { limit: options } : options
    const result: ClipboardIpcResponse<ClipboardItem[]> = await invoke('clipboard-get-history', queryOptions)
    if (result.success) return { items: result.data || [], apps: result.apps || {} }
    throw new Error(result.error || '获取剪贴板历史失败')
  },

  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardHistoryResult> {
    return this.getHistory(options)
  },

  async restore(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore', { itemId })
    return result.success
  },

  async restoreAsText(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore-as-text', { itemId })
    return result.success
  },

  async restoreOcrText(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore-ocr-text', { itemId })
    return result.success
  },

  async parseOcr(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-parse-ocr', { itemId })
    return result.success
  },

  async parseQRCode(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-parse-qrcode', { itemId })
    return result.success
  },

  async generateQRCode(itemId: string): Promise<{ success: boolean; message?: string; data?: { qrCodeDataUrl: string } }> {
    const result: ClipboardIpcResponse<{ qrCodeDataUrl: string }> = await invoke('clipboard-generate-qrcode', { itemId })
    return { success: result.success, message: result.success ? result.message || '二维码生成成功' : result.error, data: result.success ? result.data : undefined }
  },

  async deleteItem(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-delete-item', { itemId })
    return result.success
  },

  async clearHistory(): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-clear-history')
    return result.success
  },

  async getStatus(): Promise<ClipboardStatus> {
    const result: ClipboardIpcResponse<ClipboardStatus> = await invoke('clipboard-get-status')
    if (result.success) return result.data!
    throw new Error(result.error || '获取剪贴板状态失败')
  },

  async startListening(): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-start-listening')
    return result.success
  },

  async stopListening(): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-stop-listening')
    return result.success
  },

  async updateConfig(config: Partial<ClipboardManagerConfig>): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-update-config', config)
    return result.success
  }
}

export const tags = {
  async getAllTags(): Promise<TagGroup[]> {
    const result: { success: boolean; data?: TagGroup[]; error?: string } = await invoke('get-all-tags')
    if (result.success) return result.data || []
    throw new Error(result.error || '获取标签列表失败')
  },

  async createTag(name: string, color: string, description?: string): Promise<string> {
    const result: { success: boolean; data?: { tagId: string }; error?: string } = await invoke('create-tag', { name, color, description })
    if (result.success && result.data) return result.data.tagId
    throw new Error(result.error || '创建标签失败')
  },

  async updateTag(tagId: string, updates: { name?: string; color?: string; description?: string }): Promise<boolean> {
    const result: { success: boolean; error?: string } = await invoke('update-tag', { tagId, updates })
    if (result.success) return true
    throw new Error(result.error || '更新标签失败')
  },

  async deleteTag(tagId: string): Promise<boolean> {
    const result: { success: boolean; error?: string } = await invoke('delete-tag', { tagId })
    if (result.success) return true
    throw new Error(result.error || '删除标签失败')
  },

  async getTagStats(): Promise<TagStats[]> {
    const result: { success: boolean; data?: TagStats[]; error?: string } = await invoke('get-tag-stats')
    if (result.success) return result.data || []
    throw new Error(result.error || '获取标签统计失败')
  },

  async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    const result: { success: boolean; error?: string } = await invoke('bind-tag-to-record', { contentId, tagId })
    if (result.success) return true
    throw new Error(result.error || '绑定标签失败')
  },

  async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    const result: { success: boolean; error?: string } = await invoke('unbind-tag-from-record', { contentId, tagId })
    if (result.success) return true
    throw new Error(result.error || '解绑标签失败')
  }
}

export const apps: AppOperations = {
  async getAllApps(): Promise<AppInfo[]> {
    const result: { success: boolean; data?: AppInfo[]; error?: string } = await invoke('get-all-apps')
    if (result.success) return result.data || []
    throw new Error(result.error || '获取应用列表失败')
  },

  async getCacheStats(): Promise<AppCacheStats> {
    const result: { success: boolean; data?: AppCacheStats; error?: string } = await invoke('get-app-cache-stats')
    if (result.success && result.data) return result.data
    throw new Error(result.error || '获取缓存统计失败')
  }
}

export const app = {
  async restart(): Promise<boolean> {
    try {
      const { relaunch } = await import('@tauri-apps/plugin-process')
      await relaunch()
      return true
    } catch (error) {
      console.error('重启应用失败:', error)
      return false
    }
  },

  async minimizeWindow(): Promise<boolean> {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().minimize()
      return true
    } catch (error) {
      console.error('最小化窗口失败:', error)
      return false
    }
  },

  async closeWindow(): Promise<boolean> {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      await getCurrentWindow().hide()
      return true
    } catch (error) {
      console.error('关闭窗口失败:', error)
      return false
    }
  }
}
