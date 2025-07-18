import databaseService from './database-service'
import type {
  ClipboardItem,
  ClipboardContent
} from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板历史记录服务
 * 负责历史记录的查询、管理和标签操作
 */
class ClipboardHistoryService {
  /**
   * 获取历史记录
   */
  public async getHistory(limit?: number): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        limit: limit || 100,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板历史服务] 获取历史记录失败:', error)
      return []
    }
  }

  /**
   * 还原指定的剪贴板内容到当前剪贴板
   */
  public async restoreClipboard(itemId: string): Promise<{ success: boolean; content?: ClipboardContent }> {
    try {
      // 从数据库获取指定项目
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        limit: 10,
        searchText: itemId
      })

      let targetRecord: any = null
      for (const record of recordsWithTags) {
        if (record.content_id === itemId) {
          targetRecord = record
          break
        }
      }

      if (!targetRecord) {
        log.error('[剪贴板历史服务] 未找到指定的剪贴板项:', itemId)
        return { success: false }
      }

      // 重建内容对象
      const content: ClipboardContent = {}
      if (targetRecord.content_text) content.text = targetRecord.content_text
      if (targetRecord.content_html) content.html = targetRecord.content_html
      if (targetRecord.content_image) content.image = targetRecord.content_image
      if (targetRecord.content_rtf) content.rtf = targetRecord.content_rtf
      if (targetRecord.content_bookmark) content.bookmark = targetRecord.content_bookmark

      // 更新最后使用时间
      const now = Date.now()
      await databaseService.updateLastUsed({
        contentId: itemId,
        lastUsed: now
      })

      return { success: true, content }
    } catch (error) {
      log.error('[剪贴板历史服务] 还原剪贴板失败:', error)
      return { success: false }
    }
  }

  /**
   * 还原剪贴板内容为纯文本
   */
  public async restoreClipboardAsText(itemId: string): Promise<{ success: boolean; content?: ClipboardContent }> {
    try {
      // 参数验证
      if (!itemId || typeof itemId !== 'string') {
        log.error('[剪贴板历史服务] 还原为纯文本失败: 无效的项目ID:', itemId)
        return { success: false }
      }

      // 从数据库获取指定项目
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        limit: 10,
        searchText: itemId
      })

      let targetRecord: any = null
      for (const record of recordsWithTags) {
        if (record.content_id === itemId) {
          targetRecord = record
          break
        }
      }

      if (!targetRecord) {
        log.error('[剪贴板历史服务] 未找到指定的剪贴板项:', itemId)
        return { success: false }
      }

      // 检查是否有文本内容
      if (!targetRecord.content_text || !targetRecord.content_text.trim()) {
        return { success: false }
      }

      // 重建内容对象（仅包含文本）
      const content: ClipboardContent = {
        text: targetRecord.content_text
      }

      // 更新最后使用时间
      const now = Date.now()
      await databaseService.updateLastUsed({
        contentId: itemId,
        lastUsed: now
      })

      return { success: true, content }
    } catch (error) {
      log.error('[剪贴板历史服务] 还原剪贴板为纯文本时发生异常:', error, '项目ID:', itemId)
      return { success: false }
    }
  }

  /**
   * 删除指定的历史记录项
   */
  public async deleteHistoryItem(itemId: string): Promise<boolean> {
    try {
      // 直接从数据库中删除
      const success = await databaseService.deleteRecord({ contentId: itemId })

      if (!success) {
        log.error('[剪贴板历史服务] 未找到指定的剪贴板项:', itemId)
        return false
      }

      return true
    } catch (error) {
      log.error('[剪贴板历史服务] 删除剪贴板项失败:', error)
      return false
    }
  }

  /**
   * 清空所有历史记录
   */
  public async clearHistory(): Promise<void> {
    try {
      // 清空数据库中的历史记录
      await databaseService.clearAllRecords()
    } catch (error) {
      log.error('[剪贴板历史服务] 清空历史记录失败:', error)
    }
  }

  /**
   * 按标签筛选历史记录
   */
  public async getRecordsByTags(tagIds: string[]): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        tagIds,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板历史服务] 按标签筛选记录失败:', error)
      return []
    }
  }

  /**
   * 按解析类型筛选历史记录
   */
  public async getRecordsByParseType(parseType: string): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        parseType,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板历史服务] 按解析类型筛选记录失败:', error)
      return []
    }
  }

  /**
   * 搜索历史记录（支持标签和内容筛选）
   */
  public async searchHistory(options: {
    limit?: number
    offset?: number
    tagIds?: string[]
    parseType?: string
    searchText?: string
    orderBy?: 'timestamp' | 'last_used'
    orderDirection?: 'ASC' | 'DESC'
  } = {}): Promise<ClipboardItem[]> {
    try {
      const recordsWithTags = await databaseService.queryRecordsWithTags({
        limit: options.limit,
        offset: options.offset,
        tagIds: options.tagIds,
        parseType: options.parseType,
        searchText: options.searchText,
        orderBy: options.orderBy || 'last_used',
        orderDirection: options.orderDirection || 'DESC'
      })

      return ClipboardDataConverter.batchRecordsToItems(recordsWithTags)
    } catch (error) {
      log.error('[剪贴板历史服务] 搜索历史记录失败:', error)
      return []
    }
  }


}

// 导出单例实例
export default new ClipboardHistoryService()
