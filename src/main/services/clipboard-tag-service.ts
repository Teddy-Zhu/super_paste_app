import databaseService from './database-service'
import type {
  TagGroup,
  TagStats
} from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板标签管理服务
 * 负责标签的创建、更新、删除和绑定操作
 */
class ClipboardTagService {
  /**
   * 创建新标签组
   */
  public async createTagGroup(name: string, color: string, description?: string): Promise<string | null> {
    try {
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

      const success = await databaseService.createTagGroup({ tag })
      if (success) {
        return tagId
      } else {
        log.error('[剪贴板标签服务] 创建标签组失败')
        return null
      }
    } catch (error) {
      log.error('[剪贴板标签服务] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 获取所有标签组
   */
  public async getAllTagGroups(): Promise<TagGroup[]> {
    try {
      const records = await databaseService.getAllTagGroups()
      return records.map((record: any) => ClipboardDataConverter.tagRecordToTag(record))
    } catch (error) {
      log.error('[剪贴板标签服务] 获取标签组列表失败:', error)
      return []
    }
  }

  /**
   * 更新标签组
   */
  public async updateTagGroup(tagId: string, updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>>): Promise<boolean> {
    try {
      const updateData = {
        ...updates,
        updatedAt: Date.now()
      }

      const success = await databaseService.updateTagGroup({ tagId, updateData })
      return success
    } catch (error) {
      log.error('[剪贴板标签服务] 更新标签组失败:', error)
      return false
    }
  }

  /**
   * 删除标签组
   */
  public async deleteTagGroup(tagId: string): Promise<boolean> {
    try {
      const success = await databaseService.deleteTagGroup({ tagId })
      return success
    } catch (error) {
      log.error('[剪贴板标签服务] 删除标签组失败:', error)
      return false
    }
  }

  /**
   * 为历史记录绑定标签
   */
  public async bindTagToRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      const success = await databaseService.bindTagToRecord({ contentId, tagId })
      return success
    } catch (error) {
      log.error('[剪贴板标签服务] 绑定标签失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的标签绑定
   */
  public async unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean> {
    try {
      const success = await databaseService.unbindTagFromRecord({ contentId, tagId })
      return success
    } catch (error) {
      log.error('[剪贴板标签服务] 解除标签绑定失败:', error)
      return false
    }
  }

  /**
   * 获取标签统计信息
   */
  public async getTagStats(): Promise<TagStats[]> {
    try {
      return await databaseService.getTagStats()
    } catch (error) {
      log.error('[剪贴板标签服务] 获取标签统计信息失败:', error)
      return []
    }
  }
}

// 导出单例实例
export default new ClipboardTagService()
