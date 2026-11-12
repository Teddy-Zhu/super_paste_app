import { invoke } from './ipc'
import type { TagGroup, TagStats } from './types'

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
