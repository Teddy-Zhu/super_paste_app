import { invoke } from './ipc'
import type { AppOperations, AppInfo, AppCacheStats } from './types'

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
