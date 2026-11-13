import { invoke } from './ipc'
import { convertFileSrc } from '@tauri-apps/api/core'
import { OCR_MANAGE_TEXT } from '@renderer/constants/ocr'
import type {
  ClipboardOperations,
  ClipboardItem,
  ClipboardStatus,
  ClipboardManagerConfig,
  ClipboardIpcResponse,
  ClipboardHistoryQueryOptions,
  ClipboardHistoryResult
} from './types'

function resolveClipboardMedia(item: ClipboardItem): ClipboardItem {
  if (item.content.image)
    item.content.image = `${convertFileSrc('', 'clipboard-media')}${item.content.image}`
  return item
}

/**
 * 剪贴板操作封装
 */
export const clipboard: ClipboardOperations = {
  async getItem(id: string): Promise<ClipboardItem> {
    const result = await invoke('clipboard-get-item', { id })
    if (!result.success || !result.data) throw new Error(result.error || '获取剪贴板内容失败')
    return resolveClipboardMedia(result.data)
  },
  async copyColor(text: string): Promise<void> {
    const result: ClipboardIpcResponse = await invoke('clipboard-copy-color', text)
    if (!result.success) throw new Error(result.error || '复制颜色失败')
  },

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
        items: (result.data || []).map(resolveClipboardMedia),
        apps: result.apps || {},
        hasMore: result.hasMore ?? false,
        nextCursor: result.nextCursor ?? null
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
    if (!result.success) throw new Error(result.error || OCR_MANAGE_TEXT.error)
    return true
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
