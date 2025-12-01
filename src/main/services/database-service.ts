import clipboardDatabase from '../clipboard/clipboard-database'
import type { ClipboardContent, TagGroup } from '../types/clipboard-types'
import logger from '../utils/logger'


/**
 * 数据库服务
 * 统一管理所有数据库操作，简化 Worker 与数据库的交互
 */
export class DatabaseService {
  private static instance: DatabaseService

  /**
   * 获取单例实例
   */
  public static getInstance(): DatabaseService {
    if (!DatabaseService.instance) {
      DatabaseService.instance = new DatabaseService()
    }
    return DatabaseService.instance
  }

  /**
   * 统一的数据库操作执行方法
   * 使用动态方法调用，自动转发到 clipboard-database
   */
  public async executeOperation(operation: string, params?: any): Promise<any> {
    try {
      // 特殊处理：某些方法需要参数解包
      const methodMap: Record<string, () => any> = {
        initialize: () => this.initialize(),
        isConnected: () => clipboardDatabase.isConnected(),

        // 剪贴板记录操作 - 需要参数解包
        insertRecord: () => this.insertRecord(params),
        deleteRecord: () => clipboardDatabase.deleteRecord(params.contentId),
        clearAllRecords: () => clipboardDatabase.clearAllRecords(),
        updateLastUsed: () => clipboardDatabase.updateLastUsed(params.contentId, params.lastUsed),
        updateOCR: () => clipboardDatabase.updateOCR(params.contentId, params.ocrText),

        // 查询操作 - 直接传递参数对象
        queryRecordsWithTags: () => clipboardDatabase.queryRecordsWithTags(params),
        getRecordCount: () => clipboardDatabase.getRecordCount(),
        getExpiredRecordIds: () => clipboardDatabase.getExpiredRecordIds(params.expireTimestamp),
        getRecordByHash: () => clipboardDatabase.getRecordByHash(params.hash),
        getRecordByContentId: () => clipboardDatabase.getRecordByContentId(params.contentId),
        getRecordTags: () => clipboardDatabase.getRecordTags(params.contentId),
        queryRecordIdsAndHashes: () => clipboardDatabase.queryRecordIdsAndHashes(params),
        queryRecordIdsAndHashesWithoutTags: () => clipboardDatabase.queryRecordIdsAndHashesWithoutTags(params),
        getLastRecordInfo: () => clipboardDatabase.getLastRecordInfo(),
        deleteExpiredRecords: () => clipboardDatabase.deleteRecordsByTimestamp(params.expireTimestamp),

        // 标签组操作
        createTagGroup: () => this.createTagGroup(params),
        getAllTagGroups: () => clipboardDatabase.getAllTagGroups(),
        getTagGroupById: () => clipboardDatabase.getTagGroupById(params.tagId),
        updateTagGroup: () => this.updateTagGroup(params),
        deleteTagGroup: () => clipboardDatabase.deleteTagGroup(params.tagId),

        // 标签绑定操作
        bindTagToRecord: () => clipboardDatabase.bindTagToRecord(params.contentId, params.tagId),
        unbindTagFromRecord: () => clipboardDatabase.unbindTagFromRecord(params.contentId, params.tagId),
        unbindAllTagsFromRecord: () => clipboardDatabase.unbindAllTagsFromRecord(params.contentId),
        getTagStats: () => clipboardDatabase.getTagGroupStats(),
        getTagGroupStats: () => clipboardDatabase.getTagGroupStats(),

        // 应用信息操作
        insertOrUpdateApp: () => clipboardDatabase.insertOrUpdateApp(params.application, params.path, params.icon),
        getAppById: () => clipboardDatabase.getAppById(params.appId),
        getAppCacheStats: () => clipboardDatabase.getAppCacheStats()
      }

      const handler = methodMap[operation]
      if (handler) {
        return handler()
      }

      throw new Error(`未知的数据库操作: ${operation}`)
    } catch (error) {
      logger.error(`[数据库服务] 执行操作 ${operation} 失败:`, error)
      throw error
    }
  }

  /**
   * 初始化数据库
   */
  private initialize(): boolean {
    try {
      clipboardDatabase.initialize()
      return true
    } catch (error) {
      logger.error('[数据库服务] 初始化失败:', error)
      return false
    }
  }

  /**
   * 插入剪贴板记录（需要参数解包）
   */
  private insertRecord(params: {
    contentId: string
    content: ClipboardContent
    primaryType: string
    availableFormats: string[]
    timestamp: number
    lastUsed: number
    sourceApp?: string
    hash?: string
  }): boolean {
    const {
      contentId,
      content,
      primaryType,
      availableFormats,
      timestamp,
      lastUsed,
      sourceApp,
      hash
    } = params
    return clipboardDatabase.insertRecord(
      contentId,
      content,
      primaryType,
      availableFormats,
      timestamp,
      lastUsed,
      sourceApp,
      hash
    )
  }

  /**
   * 创建标签组（需要特殊处理）
   */
  private createTagGroup(
    params: { name: string; color: string; description?: string } | { tag: TagGroup }
  ): string | null {
    try {
      let tag: TagGroup

      // 检查参数格式，支持两种传参方式
      if ('tag' in params) {
        // Worker 传递的格式：{ tag: TagGroup }
        tag = params.tag
      } else {
        // 直接传递的格式：{ name, color, description }
        const tagId = `tag_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
        const now = Date.now()

        tag = {
          id: tagId,
          name: params.name,
          color: params.color,
          description: params.description,
          createdAt: now,
          updatedAt: now
        }
      }

      // 调用数据库方法创建标签组
      const success = clipboardDatabase.createTagGroup(tag)
      return success ? tag.id : null
    } catch (error) {
      logger.error('[数据库服务] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 更新标签组（需要特殊处理）
   */
  private updateTagGroup(params: {
    tagId: string
    updates?: Partial<Omit<TagGroup, 'id' | 'createdAt'>>
    updateData?: Partial<Omit<TagGroup, 'id' | 'createdAt'>>
  }): boolean {
    // 支持两种参数格式：updates 或 updateData
    const updates = params.updates || params.updateData
    if (!updates) {
      throw new Error('缺少更新数据')
    }
    return clipboardDatabase.updateTagGroup(params.tagId, updates)
  }
}

// 导出单例实例
export default DatabaseService.getInstance()
