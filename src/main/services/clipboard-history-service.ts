import clipboardDatabase from '../clipboard/clipboard-database'
import type { ClipboardItem, AppInfo } from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import log from 'electron-log'

/**
 * 剪贴板历史记录服务
 * 负责历史记录的查询、管理和标签操作
 */
class ClipboardHistoryService {
  /**
   * 删除指定的历史记录项
   */
  public async deleteHistoryItem(itemId: string): Promise<boolean> {
    try {
      // 直接从数据库中删除
      const success = await clipboardDatabase.deleteRecord(itemId)

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
      await clipboardDatabase.clearAllRecords()
    } catch (error) {
      log.error('[剪贴板历史服务] 清空历史记录失败:', error)
    }
  }

  /**
   * 搜索历史记录（支持标签和内容筛选）
   * 返回包含历史记录和所有应用信息的结构
   */
  public async searchHistory(
    options: {
      limit?: number
      offset?: number
      tagIds?: string[]
      primaryType?: string
      searchText?: string
      orderBy?: 'timestamp' | 'last_used'
      orderDirection?: 'ASC' | 'DESC'
    } = {}
  ): Promise<{
    items: ClipboardItem[]
    apps: Record<string, AppInfo>
  }> {
    try {
      const recordsWithTags = await clipboardDatabase.queryRecordsWithTags({
        limit: options.limit,
        offset: options.offset,
        tagIds: options.tagIds,
        primaryType: options.primaryType,
        searchText: options.searchText,
        orderBy: options.orderBy || 'last_used',
        orderDirection: options.orderDirection || 'DESC'
      })

      const items = ClipboardDataConverter.batchRecordsToItems(recordsWithTags)

      // 收集所有应用信息
      const apps: Record<string, AppInfo> = {}
      for (const record of recordsWithTags) {
        if (record.appInfo) {
          const appInfo = ClipboardDataConverter.appRecordToAppInfo(record.appInfo)
          apps[appInfo.id] = appInfo
        }
      }

      return { items, apps }
    } catch (error) {
      log.error('[剪贴板历史服务] 搜索历史记录失败:', error)
      return { items: [], apps: {} }
    }
  }

  /**
   * 根据内容ID获取单条剪贴板历史记录
   * 返回转换后的剪贴板项和可选的应用信息
   */
  public async getHistoryItemById(
    contentId: string
  ): Promise<{ item: ClipboardItem; app?: AppInfo } | null> {
    try {
      const record = clipboardDatabase.getRecordByContentId(contentId)
      if (!record) {
        return null
      }

      const tagRecords = clipboardDatabase.getRecordTags(contentId)
      const tags = tagRecords.map((tag) => ClipboardDataConverter.tagRecordToTag(tag))
      const item = ClipboardDataConverter.recordToItem(record, tags)

      const appRecord = record.source_app
        ? clipboardDatabase.getAppById(record.source_app)
        : null
      const app = appRecord ? ClipboardDataConverter.appRecordToAppInfo(appRecord) : undefined

      return { item, app }
    } catch (error) {
      log.error('[剪贴板历史服务] 获取剪贴板项失败:', error)
      return null
    }
  }
}

// 导出单例实例
export default new ClipboardHistoryService()
