/**
 * 渲染进程高级 API 封装
 * 提供更便捷的方法来与主进程通信
 */

import { 
  on, 
  onSimple, 
  once, 
  onceSimple, 
  send, 
  invoke, 
  sendSync, 
  removeListener, 
  removeAllListeners 
} from './ipc'

import type {
  StoreOperations,
  RendererAPI,
  ClipboardOperations,
  ClipboardItem,
  ClipboardStatus,
  ClipboardManagerConfig,
  ClipboardIpcResponse,
  ClipboardHistoryQueryOptions,
  TagGroup,
  TagStats,
  AppInfo,
  AppCacheStats,
  AppOperations
} from './types'

/**
 * 存储操作封装
 */
export const store: StoreOperations = {
  async get<T = any>(key: string, defaultValue?: T): Promise<T> {
    const result = await invoke('store-get', { key, defaultValue })
    if (result.success) {
      return result.data ?? defaultValue
    }
    throw new Error(result.error || '获取存储值失败')
  },

  async set<T = any>(key: string, value: T): Promise<void> {
    const result = await invoke('store-set', { key, value })
    if (!result.success) {
      throw new Error(result.error || '设置存储值失败')
    }
  },
}

/**
 * 剪贴板操作封装
 */
export const clipboard: ClipboardOperations = {
  async getHistory(options?: ClipboardHistoryQueryOptions | number): Promise<ClipboardItem[]> {
    // 向后兼容：如果传入的是数字，转换为 limit 参数
    const queryOptions = typeof options === 'number' ? { limit: options } : options

    const result: ClipboardIpcResponse<ClipboardItem[]> = await invoke('clipboard-get-history', queryOptions)
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取剪贴板历史失败')
  },

  async searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardItem[]> {
    const result: ClipboardIpcResponse<ClipboardItem[]> = await invoke('clipboard-get-history', options)
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '搜索剪贴板历史失败')
  },

  async restore(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore', itemId)
    return result.success
  },

  async restoreAsText(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore-as-text', itemId)
    return result.success
  },

  async deleteItem(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-delete-item', itemId)
    return result.success
  },

  async clearHistory(): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-clear-history')
    return result.success
  },

  async getStatus(): Promise<ClipboardStatus> {
    const result: ClipboardIpcResponse<ClipboardStatus> = await invoke('clipboard-get-status')
    if (result.success) {
      return result.data!
    }
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
  },
}


/**
 * 工具函数：安全的 invoke 调用
 */
export async function safeInvoke<T>(
  channel: string, 
  data?: any, 
  defaultValue?: T
): Promise<T | undefined> {
  try {
    return await invoke<T>(channel, data)
  } catch (error) {
    return defaultValue
  }
}

/**
 * 标签操作封装
 */
export const tags = {
  async getAllTags(): Promise<TagGroup[]> {
    const result: { success: boolean; data?: TagGroup[]; error?: string } = await invoke('get-all-tags')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取标签列表失败')
  },

  async createTag(name: string, color: string, description?: string): Promise<string> {
    const result: { success: boolean; data?: { tagId: string }; message?: string; error?: string } = await invoke('create-tag', { name, color, description })
    if (result.success && result.data) {
      return result.data.tagId
    }
    throw new Error(result.error || '创建标签失败')
  },

  async updateTag(tagId: string, updates: { name?: string; color?: string; description?: string }): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke('update-tag', { tagId, updates })
    if (result.success) {
      return true
    }
    throw new Error(result.error || '更新标签失败')
  },

  async deleteTag(tagId: string): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke('delete-tag', tagId)
    if (result.success) {
      return true
    }
    throw new Error(result.error || '删除标签失败')
  },

  async getTagStats(): Promise<TagStats[]> {
    const result: { success: boolean; data?: TagStats[]; error?: string } = await invoke('get-tag-stats')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取标签统计信息失败')
  }
}

/**
 * 应用信息操作封装
 */
export const apps: AppOperations = {
  async getAllApps(): Promise<AppInfo[]> {
    const result: { success: boolean; data?: AppInfo[]; error?: string } = await invoke('get-all-apps')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取应用列表失败')
  },

  async getCacheStats(): Promise<AppCacheStats> {
    const result: { success: boolean; data?: AppCacheStats; error?: string } = await invoke('get-app-cache-stats')
    if (result.success && result.data) {
      return result.data
    }
    throw new Error(result.error || '获取应用缓存统计失败')
  }
}

/**
 * 工具函数：带超时的 invoke 调用
 */
export async function invokeWithTimeout<T>(
  channel: string,
  data?: any,
  timeout: number = 5000
): Promise<T> {
  return Promise.race([
    invoke<T>(channel, data),
    new Promise<never>((_, reject) => 
      setTimeout(() => reject(new Error(`IPC 调用超时: ${channel}`)), timeout)
    )
  ])
}

/**
 * 工具函数：批量移除监听器
 */
export function removeListeners(listeners: Array<() => void>): void {
  listeners.forEach(remove => remove())
}

/**
 * 完整的渲染进程 API
 */
export const rendererAPI: RendererAPI = {
  // 存储操作
  ...store,

  // 剪贴板操作
  clipboard,

  // 标签操作
  tags,

  // 应用信息操作
  apps,

  // 原始 IPC 方法
  ipc: {
    on,
    onSimple,
    once,
    onceSimple,
    send,
    invoke,
    sendSync,
    removeListener,
    removeAllListeners
  } as any,
}

// 默认导出
export default rendererAPI
