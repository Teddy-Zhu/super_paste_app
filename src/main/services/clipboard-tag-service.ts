import clipboardDatabase from '../infrastructure/database/clipboard-database'
import type { TagGroup, TagStats } from '../features/clipboard/types/clipboard-types'
import { ClipboardDataConverter } from '../features/clipboard/types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板标签管理服务
 * 负责标签的创建、更新、删除和绑定操作
 */
class ClipboardTagService {
  /**
   * 通用执行辅助函数，统一错误处理逻辑
   */
  private async execute<T>(
    operation: () => Promise<T>,
    errorMessage: string,
    fallback: T
  ): Promise<T> {
    try {
      return await operation()
    } catch (error) {
      log.error(`[剪贴板标签服务] ${errorMessage}:`, error)
      return fallback
    }
  }

  /**
   * 创建新标签组
   */
  public async createTagGroup(
    name: string,
    color: string,
    description?: string
  ): Promise<string | null> {
    return this.execute(
      async () => {
        const tagId = ClipboardDataConverter.generateTagId()
        const now = Date.now()

        const tag: TagGroup = {
          id: tagId,
          name,
          color,
          description,
          createdAt: now,
          updatedAt: now
        }

        const success = clipboardDatabase.createTagGroup(tag)
        if (success) {
          return tagId
        } else {
          log.error('[剪贴板标签服务] 创建标签组失败')
          return null
        }
      },
      '创建标签组失败',
      null
    )
  }

  /**
   * 获取所有标签组
   */
  public async getAllTagGroups(): Promise<TagGroup[]> {
    return this.execute(
      async () => {
        const records = clipboardDatabase.getAllTagGroups()
        return records.map((record) => ClipboardDataConverter.tagRecordToTag(record))
      },
      '获取标签组列表失败',
      []
    )
  }

  /**
   * 更新标签组
   */
  public async updateTagGroup(
    tagId: string,
    updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>>
  ): Promise<boolean> {
    return this.execute(
      async () => {
        const updateData = {
          ...updates,
          updatedAt: Date.now()
        }

        const success = clipboardDatabase.updateTagGroup(tagId, updateData)
        return success
      },
      '更新标签组失败',
      false
    )
  }

  /**
   * 删除标签组
   */
  public async deleteTagGroup(tagId: string): Promise<boolean> {
    return this.execute(
      async () => {
        const success = clipboardDatabase.deleteTagGroup(tagId)
        return success
      },
      '删除标签组失败',
      false
    )
  }

  /**
   * 为历史记录绑定标签
   */
  public async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    return this.execute(
      async () => {
        const success = clipboardDatabase.bindTagToRecord(contentId, tagId)
        return success
      },
      '绑定标签失败',
      false
    )
  }

  /**
   * 解除历史记录的标签绑定
   */
  public async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    return this.execute(
      async () => {
        const success = clipboardDatabase.unbindTagFromRecord(contentId, tagId)
        return success
      },
      '解除标签绑定失败',
      false
    )
  }

  /**
   * 解除历史记录的所有标签绑定
   */
  public async unbindAllTagsFromRecord(contentId: string): Promise<boolean> {
    return this.execute(
      async () => {
        const success = clipboardDatabase.unbindAllTagsFromRecord(contentId)
        return success
      },
      '解除所有标签绑定失败',
      false
    )
  }

  /**
   * 获取标签统计信息
   */
  public async getTagStats(): Promise<TagStats[]> {
    return this.execute(
      async () => {
        return clipboardDatabase.getTagGroupStats()
      },
      '获取标签统计信息失败',
      []
    )
  }
}

// 导出单例实例
export default new ClipboardTagService()
