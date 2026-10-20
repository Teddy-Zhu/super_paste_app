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
  RendererAPI,
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

/**
 * 剪贴板操作封装
 */
export const clipboard: ClipboardOperations = {
  async getHistory(
    options?: ClipboardHistoryQueryOptions | number
  ): Promise<ClipboardHistoryResult> {
    // 向后兼容：如果传入的是数字，转换为 limit 参数
    const queryOptions = typeof options === 'number' ? { limit: options } : options

    const result: ClipboardIpcResponse<ClipboardItem[]> = await invoke(
      'clipboard-get-history',
      queryOptions
    )
    if (result.success) {
      return {
        items: result.data || [],
        apps: result.apps || {}
      }
    }
    throw new Error(result.error || '获取剪贴板历史失败')
  },

  // searchHistory 现在是 getHistory 的别名
  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardHistoryResult> {
    return this.getHistory(options)
  },

  async restore(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore', itemId)
    return result.success
  },

  async restoreAsText(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore-as-text', itemId)
    return result.success
  },

  async restoreOcrText(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-restore-ocr-text', itemId)
    return result.success
  },

  async parseOcr(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-parse-ocr', itemId)
    return result.success
  },

  async parseQRCode(itemId: string): Promise<boolean> {
    const result: ClipboardIpcResponse = await invoke('clipboard-parse-qrcode', itemId)
    return result.success
  },

  async generateQRCode(
    itemId: string
  ): Promise<{ success: boolean; message?: string; data?: { qrCodeDataUrl: string } }> {
    const result: ClipboardIpcResponse<{ qrCodeDataUrl: string }> = await invoke(
      'clipboard-generate-qrcode',
      itemId
    )
    return {
      success: result.success,
      message: result.success ? result.message || '二维码生成成功' : result.error,
      data: result.success ? result.data : undefined
    }
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
  }
}

/**
 * 标签操作封装
 */
export const tags = {
  async getAllTags(): Promise<TagGroup[]> {
    const result: { success: boolean; data?: TagGroup[]; error?: string } =
      await invoke('get-all-tags')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取标签列表失败')
  },

  async createTag(name: string, color: string, description?: string): Promise<string> {
    const result: { success: boolean; data?: { tagId: string }; message?: string; error?: string } =
      await invoke('create-tag', { name, color, description })
    if (result.success && result.data) {
      return result.data.tagId
    }
    throw new Error(result.error || '创建标签失败')
  },

  async updateTag(
    tagId: string,
    updates: { name?: string; color?: string; description?: string }
  ): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke(
      'update-tag',
      { tagId, updates }
    )
    if (result.success) {
      return true
    }
    throw new Error(result.error || '更新标签失败')
  },

  async deleteTag(tagId: string): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke(
      'delete-tag',
      tagId
    )
    if (result.success) {
      return true
    }
    throw new Error(result.error || '删除标签失败')
  },

  async getTagStats(): Promise<TagStats[]> {
    const result: { success: boolean; data?: TagStats[]; error?: string } =
      await invoke('get-tag-stats')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取标签统计信息失败')
  },

  async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke(
      'bind-tag-to-record',
      { contentId, tagId }
    )
    if (result.success) {
      return true
    }
    throw new Error(result.error || '绑定标签失败')
  },

  async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    const result: { success: boolean; message?: string; error?: string } = await invoke(
      'unbind-tag-from-record',
      { contentId, tagId }
    )
    if (result.success) {
      return true
    }
    throw new Error(result.error || '解除标签绑定失败')
  }
}

/**
 * 应用信息操作封装
 */
export const apps: AppOperations = {
  async getAllApps(): Promise<AppInfo[]> {
    const result: { success: boolean; data?: AppInfo[]; error?: string } =
      await invoke('get-all-apps')
    if (result.success) {
      return result.data || []
    }
    throw new Error(result.error || '获取应用列表失败')
  },

  async getCacheStats(): Promise<AppCacheStats> {
    const result: { success: boolean; data?: AppCacheStats; error?: string } =
      await invoke('get-app-cache-stats')
    if (result.success && result.data) {
      return result.data
    }
    throw new Error(result.error || '获取应用缓存统计失败')
  }
}

/**
 * 应用控制操作封装
 */
export const app = {
  async restart(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:restart')
      return result.success
    } catch (error) {
      console.error('重启应用失败:', error)
      return false
    }
  },

  async minimizeWindow(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:window-minimize')
      return result.success
    } catch (error) {
      console.error('最小化窗口失败:', error)
      return false
    }
  },

  async closeWindow(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:window-close')
      return result.success
    } catch (error) {
      console.error('关闭窗口失败:', error)
      return false
    }
  }
}

/**
 * 完整的渲染进程 API
 */
export const rendererAPI: RendererAPI = {
  // 剪贴板操作
  clipboard,

  // 标签操作
  tags,

  // 应用信息操作
  apps,

  // 应用控制操作
  app,

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
  }
}

// 默认导出
export default rendererAPI
