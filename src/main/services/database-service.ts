import clipboardDatabase from '../clipboard-database'
import type {
  ClipboardContent,
  TagGroup,
  ClipboardHistoryRecord,
  TagGroupRecord,
  AppInfoRecord,
  QueryOptions
} from '../types/clipboard-types'
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
   * 初始化数据库
   */
  public initialize(): boolean {
    try {
      clipboardDatabase.initialize()
      return true
    } catch (error) {
      logger.error('[数据库服务] 初始化失败:', error)
      return false
    }
  }

  /**
   * 检查数据库连接状态
   */
  public isConnected(): boolean {
    return clipboardDatabase.isConnected()
  }

  // ==================== 剪贴板记录操作 ====================

  /**
   * 插入剪贴板记录
   */
  public insertRecord(params: {
    contentId: string
    content: ClipboardContent
    primaryType: string
    availableFormats: string[]
    timestamp: number
    lastUsed: number
    parseType: string
    sourceAppId?: string
  }): boolean {
    const { contentId, content, primaryType, availableFormats, timestamp, lastUsed, parseType, sourceAppId } = params
    return clipboardDatabase.insertRecord(
      contentId,
      content,
      primaryType,
      availableFormats,
      timestamp,
      lastUsed,
      parseType,
      sourceAppId
    )
  }

  /**
   * 删除剪贴板记录
   */
  public deleteRecord(params: { contentId: string }): boolean {
    return clipboardDatabase.deleteRecord(params.contentId)
  }

  /**
   * 清空所有记录
   */
  public clearAllRecords(): boolean {
    return clipboardDatabase.clearAllRecords()
  }

  /**
   * 更新最后使用时间
   */
  public updateLastUsed(params: { contentId: string, lastUsed: number }): boolean {
    return clipboardDatabase.updateLastUsed(params.contentId, params.lastUsed)
  }

  /**
   * 查询带标签信息的记录
   */
  public queryRecordsWithTags(options: QueryOptions = {}): Array<ClipboardHistoryRecord & { tags: TagGroupRecord[], appInfo?: AppInfoRecord }> {
    return clipboardDatabase.queryRecordsWithTags(options)
  }

  /**
   * 获取记录总数
   */
  public getRecordCount(): number {
    return clipboardDatabase.getRecordCount()
  }

  /**
   * 获取过期记录ID列表
   */
  public getExpiredRecordIds(params: { expireTimestamp: number }): string[] {
    return clipboardDatabase.getExpiredRecordIds(params.expireTimestamp)
  }

  /**
   * 删除过期记录
   */
  public deleteExpiredRecords(params: { expireTimestamp: number }): number {
    return clipboardDatabase.deleteRecordsByTimestamp(params.expireTimestamp)
  }

  // ==================== 标签组操作 ====================

  /**
   * 创建标签组
   */
  public createTagGroup(params: { name: string, color: string, description?: string }): string | null {
    try {
      // 生成标签ID和时间戳
      const tagId = `tag_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
      const now = Date.now()

      // 构造TagGroup对象
      const tag: TagGroup = {
        id: tagId,
        name: params.name,
        color: params.color,
        description: params.description,
        createdAt: now,
        updatedAt: now
      }

      // 调用数据库方法创建标签组
      const success = clipboardDatabase.createTagGroup(tag)
      return success ? tagId : null
    } catch (error) {
      logger.error('[数据库服务] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 获取所有标签组
   */
  public getAllTagGroups(): TagGroupRecord[] {
    return clipboardDatabase.getAllTagGroups()
  }

  /**
   * 根据ID获取标签组
   */
  public getTagGroupById(params: { tagId: string }): TagGroupRecord | null {
    return clipboardDatabase.getTagGroupById(params.tagId)
  }

  /**
   * 更新标签组
   */
  public updateTagGroup(params: { tagId: string, updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>> }): boolean {
    return clipboardDatabase.updateTagGroup(params.tagId, params.updates)
  }

  /**
   * 删除标签组
   */
  public deleteTagGroup(params: { tagId: string }): boolean {
    return clipboardDatabase.deleteTagGroup(params.tagId)
  }

  /**
   * 绑定标签到记录
   */
  public bindTagToRecord(params: { contentId: string, tagId: string }): boolean {
    return clipboardDatabase.bindTagToRecord(params.contentId, params.tagId)
  }

  /**
   * 解除标签绑定
   */
  public unbindTagFromRecord(params: { contentId: string, tagId: string }): boolean {
    return clipboardDatabase.unbindTagFromRecord(params.contentId, params.tagId)
  }

  /**
   * 获取标签统计信息
   */
  public getTagStats(): Array<{ tagId: string, tagName: string, recordCount: number }> {
    return clipboardDatabase.getTagGroupStats()
  }

  // ==================== 应用信息操作 ====================

  /**
   * 插入或更新应用信息
   */
  public insertOrUpdateApp(params: { application: string, path: string, icon?: string }): string | null {
    return clipboardDatabase.insertOrUpdateApp(params.application, params.path, params.icon)
  }

  /**
   * 获取应用信息
   */
  public getAppById(params: { appId: string }): AppInfoRecord | null {
    return clipboardDatabase.getAppById(params.appId)
  }

  /**
   * 获取应用缓存统计信息
   */
  public getAppCacheStats(): { cacheSize: number, maxCacheSize: number } {
    return clipboardDatabase.getAppCacheStats()
  }

  /**
   * 统一的数据库操作执行方法
   * 用于 Worker 通过字符串方法名调用数据库操作
   */
  public async executeOperation(operation: string, params?: any): Promise<any> {
    try {
      switch (operation) {
        case 'initialize':
          return this.initialize()
        case 'isConnected':
          return this.isConnected()
        case 'insertRecord':
          return this.insertRecord(params)
        case 'deleteRecord':
          return this.deleteRecord(params)
        case 'clearAllRecords':
          return this.clearAllRecords()
        case 'updateLastUsed':
          return this.updateLastUsed(params)
        case 'queryRecordsWithTags':
          return this.queryRecordsWithTags(params)
        case 'getRecordCount':
          return this.getRecordCount()
        case 'getExpiredRecordIds':
          return this.getExpiredRecordIds(params)
        case 'deleteExpiredRecords':
          return this.deleteExpiredRecords(params)
        case 'createTagGroup':
          return this.createTagGroup(params)
        case 'getAllTagGroups':
          return this.getAllTagGroups()
        case 'getTagGroupById':
          return this.getTagGroupById(params)
        case 'updateTagGroup':
          return this.updateTagGroup(params)
        case 'deleteTagGroup':
          return this.deleteTagGroup(params)
        case 'bindTagToRecord':
          return this.bindTagToRecord(params)
        case 'unbindTagFromRecord':
          return this.unbindTagFromRecord(params)
        case 'getTagStats':
          return this.getTagStats()
        case 'getTagGroupStats':
          return this.getTagStats()
        case 'insertOrUpdateApp':
          return this.insertOrUpdateApp(params)
        case 'getAppById':
          return this.getAppById(params)
        case 'getAppCacheStats':
          return this.getAppCacheStats()
        default:
          throw new Error(`未知的数据库操作: ${operation}`)
      }
    } catch (error) {
      logger.error(`[数据库服务] 执行操作 ${operation} 失败:`, error)
      throw error
    }
  }
}

// 导出单例实例
export default DatabaseService.getInstance()
