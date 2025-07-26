import Database from 'better-sqlite3'
import { app } from 'electron'
import { join } from 'path'
import type {
  ClipboardContent,
  ClipboardHistoryRecord,
  QueryOptions,
  TagGroup,
  TagGroupRecord,
  TagQueryOptions,
  TagStats,
  AppInfoRecord
} from '../types/clipboard-types'
import { ClipboardDataConverter } from '../types/clipboard-types'
import logger from '../utils/logger'

/**
 * 剪贴板数据库管理器
 * 使用 better-sqlite3 实现剪贴板历史记录的持久化存储
 */
class ClipboardDatabase {
  private static instance: ClipboardDatabase
  private db: Database.Database | null = null
  private dbPath: string

  // 应用信息内存缓存
  private appCache: Map<string, AppInfoRecord> = new Map()
  private appCacheByAppPath: Map<string, AppInfoRecord> = new Map()
  private appCacheMaxSize: number = 1000 // 最大缓存数量

  private constructor() {
    // 数据库文件保存在应用的 userData 目录中
    const userDataPath = app.getPath('userData')
    this.dbPath = join(userDataPath, 'clipboard-history.db')
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ClipboardDatabase {
    if (!ClipboardDatabase.instance) {
      ClipboardDatabase.instance = new ClipboardDatabase()
    }
    return ClipboardDatabase.instance
  }

  /**
   * 初始化数据库连接和表结构
   */
  public initialize(): void {
    try {
      // 创建数据库连接
      this.db = new Database(this.dbPath)
      
      // 设置数据库配置
      this.db.pragma('journal_mode = WAL') // 使用 WAL 模式提高并发性能
      this.db.pragma('synchronous = NORMAL') // 平衡性能和安全性
      this.db.pragma('cache_size = 1000') // 设置缓存大小
      this.db.pragma('temp_store = memory') // 临时数据存储在内存中

      // 创建表结构
      this.createTables()

      // 创建索引
      this.createIndexes()

      // 预加载应用信息缓存
      this.preloadAppCache()


    } catch (error) {
      logger.error('[剪贴板数据库] 数据库初始化失败:', error)
      throw error
    }
  }

  /**
   * 创建数据库表结构
   */
  private createTables(): void {
    if (!this.db) throw new Error('数据库未初始化')

    // 创建应用信息表
    const createAppsTableSQL = `
      CREATE TABLE IF NOT EXISTS apps (
        id TEXT PRIMARY KEY,
        application TEXT NOT NULL,
        path TEXT NOT NULL,
        icon TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE(application, path)
      )
    `

    // 创建剪贴板历史记录表
    const createClipboardHistoryTableSQL = `
      CREATE TABLE IF NOT EXISTS clipboard_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        content_id TEXT NOT NULL UNIQUE,
        hash TEXT NOT NULL,
        content_text TEXT,
        content_html TEXT,
        content_image BLOB,
        content_rtf TEXT,
        content_files TEXT,
        primary_type TEXT NOT NULL,
        available_formats TEXT NOT NULL,
        timestamp INTEGER NOT NULL,
        last_used INTEGER NOT NULL,
        source_app TEXT,
        created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY (source_app) REFERENCES apps (id) ON DELETE SET NULL
      )
    `

    // 创建标签组表
    const createTagGroupsTableSQL = `
      CREATE TABLE IF NOT EXISTS tag_groups (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        color TEXT NOT NULL,
        description TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
      )
    `

    // 创建剪贴板历史记录与标签的关联表
    const createClipboardHistoryTagsTableSQL = `
      CREATE TABLE IF NOT EXISTS clipboard_history_tags (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        content_id TEXT NOT NULL,
        tag_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        FOREIGN KEY (content_id) REFERENCES clipboard_history (content_id) ON DELETE CASCADE,
        FOREIGN KEY (tag_id) REFERENCES tag_groups (id) ON DELETE CASCADE,
        UNIQUE(content_id, tag_id)
      )
    `

    this.db.exec(createAppsTableSQL)
    this.db.exec(createClipboardHistoryTableSQL)
    this.db.exec(createTagGroupsTableSQL)
    this.db.exec(createClipboardHistoryTagsTableSQL)

  }



  /**
   * 创建数据库索引以优化查询性能
   */
  private createIndexes(): void {
    if (!this.db) throw new Error('数据库未初始化')

    const indexes = [
      // 应用表索引
      'CREATE INDEX IF NOT EXISTS idx_apps_application ON apps(application)',
      'CREATE INDEX IF NOT EXISTS idx_apps_path ON apps(path)',
      'CREATE INDEX IF NOT EXISTS idx_apps_application_path ON apps(application, path)',

      // 剪贴板历史记录表索引
      'CREATE INDEX IF NOT EXISTS idx_content_id ON clipboard_history(content_id)',
      'CREATE INDEX IF NOT EXISTS idx_hash ON clipboard_history(hash)',
      'CREATE INDEX IF NOT EXISTS idx_timestamp ON clipboard_history(timestamp DESC)',
      'CREATE INDEX IF NOT EXISTS idx_last_used ON clipboard_history(last_used DESC)',
      'CREATE INDEX IF NOT EXISTS idx_primary_type ON clipboard_history(primary_type)',
      'CREATE INDEX IF NOT EXISTS idx_source_app ON clipboard_history(source_app)',
      'CREATE INDEX IF NOT EXISTS idx_timestamp_type ON clipboard_history(timestamp DESC, primary_type)',
      'CREATE INDEX IF NOT EXISTS idx_content_text ON clipboard_history(content_text)',
      'CREATE INDEX IF NOT EXISTS idx_content_html ON clipboard_history(content_html)',
      'CREATE INDEX IF NOT EXISTS idx_content_files ON clipboard_history(content_files)',

      // 标签组表索引
      'CREATE INDEX IF NOT EXISTS idx_tag_groups_name ON tag_groups(name)',
      'CREATE INDEX IF NOT EXISTS idx_tag_groups_created_at ON tag_groups(created_at)',

      // 关联表索引
      'CREATE INDEX IF NOT EXISTS idx_clipboard_history_tags_content_id ON clipboard_history_tags(content_id)',
      'CREATE INDEX IF NOT EXISTS idx_clipboard_history_tags_tag_id ON clipboard_history_tags(tag_id)',
      'CREATE INDEX IF NOT EXISTS idx_clipboard_history_tags_created_at ON clipboard_history_tags(created_at)'
    ]

    indexes.forEach(indexSQL => {
      this.db!.exec(indexSQL)
    })
  }

  /**
   * 插入新的剪贴板记录
   */
  public insertRecord(
    contentId: string,
    content: ClipboardContent,
    primaryType: string,
    availableFormats: string[],
    timestamp: number,
    lastUsed: number,
    sourceApp?: string,
    hash?: string
  ): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      // 如果没有提供 hash，则生成一个
      const contentHash = hash || ClipboardDataConverter.generateContentHash(content)

      const insertSQL = `
        INSERT OR REPLACE INTO clipboard_history (
          content_id, hash, content_text, content_html, content_image,
          content_rtf, content_files, primary_type, available_formats,
          timestamp, last_used, source_app, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
      `

      const stmt = this.db.prepare(insertSQL)
      const result = stmt.run(
        contentId,
        contentHash,
        content.text || null,
        content.html || null,
        content.image || null,
        content.rtf || null,
        content.files ? JSON.stringify(content.files) : null,
        primaryType,
        JSON.stringify(availableFormats),
        timestamp,
        lastUsed,
        sourceApp || null
      )

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 插入记录失败:', error)
      return false
    }
  }

  /**
   * 更新记录的最后使用时间
   */
  public updateLastUsed(contentId: string, lastUsed: number): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const updateSQL = `
        UPDATE clipboard_history 
        SET last_used = ?, updated_at = CURRENT_TIMESTAMP 
        WHERE content_id = ?
      `

      const stmt = this.db.prepare(updateSQL)
      const result = stmt.run(lastUsed, contentId)

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 更新最后使用时间失败:', error)
      return false
    }
  }

  /**
   * 查询历史记录
   */
  public queryRecords(options: QueryOptions = {}): ClipboardHistoryRecord[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const {
        limit = 100,
        offset = 0,
        orderBy = 'last_used',
        orderDirection = 'DESC',
        tagIds,
        primaryType,
        searchText
      } = options

      let querySQL = 'SELECT DISTINCT ch.* FROM clipboard_history ch'
      const params: any[] = []
      const whereConditions: string[] = []

      // 如果有标签过滤条件
      if (tagIds && tagIds.length > 0) {
        querySQL += ' INNER JOIN clipboard_history_tags cht ON ch.content_id = cht.content_id'
        const placeholders = tagIds.map(() => '?').join(',')
        whereConditions.push(`cht.tag_id IN (${placeholders})`)
        params.push(...tagIds)
      }

      // 如果有主要类型过滤条件
      if (primaryType) {
        whereConditions.push('ch.primary_type = ?')
        params.push(primaryType)
      }

      // 如果有内容搜索条件
      if (searchText && searchText.trim()) {
        const searchPattern = `%${searchText.trim()}%`
        whereConditions.push('(ch.content_text LIKE ?)')
        params.push(searchPattern)
      }

      // 添加WHERE子句
      if (whereConditions.length > 0) {
        querySQL += ` WHERE ${whereConditions.join(' AND ')}`
      }

      // 添加ORDER BY子句
      querySQL += ` ORDER BY ch.${orderBy} ${orderDirection}`

      // 添加LIMIT和OFFSET
      querySQL += ' LIMIT ? OFFSET ?'
      params.push(limit, offset)

      const stmt = this.db.prepare(querySQL)
      return stmt.all(...params) as ClipboardHistoryRecord[]
    } catch (error) {
      logger.error('[剪贴板数据库] 查询记录失败:', error)
      return []
    }
  }

  /**
   * 根据内容ID查询记录
   */
  public getRecordByContentId(contentId: string): ClipboardHistoryRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const querySQL = 'SELECT * FROM clipboard_history WHERE content_id = ?'
      const stmt = this.db.prepare(querySQL)
      return stmt.get(contentId) as ClipboardHistoryRecord | null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据内容ID查询记录失败:', error)
      return null
    }
  }

  /**
   * 根据内容哈希查询记录
   */
  public getRecordByHash(hash: string): ClipboardHistoryRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const querySQL = 'SELECT * FROM clipboard_history WHERE hash = ? ORDER BY last_used DESC LIMIT 1'
      const stmt = this.db.prepare(querySQL)
      return stmt.get(hash) as ClipboardHistoryRecord | null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据哈希查询记录失败:', error)
      return null
    }
  }

  /**
   * 轻量级查询：只获取记录的 content_id 和 hash
   * 用于缓存初始化，避免加载完整记录数据
   */
  public queryRecordIdsAndHashes(options: QueryOptions = {}): Array<{ content_id: string, hash: string }> {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const {
        limit = 10000,
        offset = 0,
        orderBy = 'last_used',
        orderDirection = 'DESC'
      } = options

      // 只查询 content_id 和 hash 字段
      let querySQL = 'SELECT content_id, hash FROM clipboard_history'
      const params: any[] = []

      // 添加ORDER BY子句
      querySQL += ` ORDER BY ${orderBy} ${orderDirection}`

      // 添加LIMIT和OFFSET
      querySQL += ' LIMIT ? OFFSET ?'
      params.push(limit, offset)

      const stmt = this.db.prepare(querySQL)
      const results = stmt.all(...params) as Array<{ content_id: string, hash: string }>

      // 过滤掉没有 hash 的记录
      return results.filter(record => record.hash)
    } catch (error) {
      logger.error('[剪贴板数据库] 查询记录ID和哈希失败:', error)
      return []
    }
  }

  /**
   * 轻量级查询：获取最后使用的记录的基本信息（content_id 和 hash）
   */
  public getLastRecordInfo(): { content_id: string, hash: string } | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const querySQL = 'SELECT content_id, hash FROM clipboard_history ORDER BY last_used DESC LIMIT 1'
      const stmt = this.db.prepare(querySQL)
      const result = stmt.get() as { content_id: string, hash: string } | undefined

      return result && result.hash ? result : null
    } catch (error) {
      logger.error('[剪贴板数据库] 获取最后记录信息失败:', error)
      return null
    }
  }

  /**
   * 删除指定的记录
   */
  public deleteRecord(contentId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const deleteSQL = 'DELETE FROM clipboard_history WHERE content_id = ?'
      const stmt = this.db.prepare(deleteSQL)
      const result = stmt.run(contentId)

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 删除记录失败:', error)
      return false
    }
  }

  /**
   * 清空所有历史记录
   */
  public clearAllRecords(): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      // 首先清空所有标签绑定
      const deleteTagsSQL = 'DELETE FROM clipboard_history_tags'
      const deleteTagsStmt = this.db.prepare(deleteTagsSQL)
      deleteTagsStmt.run()

      // 然后清空历史记录
      const deleteSQL = 'DELETE FROM clipboard_history'
      const stmt = this.db.prepare(deleteSQL)
      stmt.run()

      // 重置自增ID
      this.db.exec('DELETE FROM sqlite_sequence WHERE name = "clipboard_history"')

      return true
    } catch (error) {
      logger.error('[剪贴板数据库] 清空历史记录失败:', error)
      return false
    }
  }

  /**
   * 删除超过指定数量的旧记录（保留最新的N条记录）
   */
  public deleteOldRecords(keepCount: number): number {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const deleteSQL = `
        DELETE FROM clipboard_history 
        WHERE id NOT IN (
          SELECT id FROM clipboard_history 
          ORDER BY last_used DESC 
          LIMIT ?
        )
      `

      const stmt = this.db.prepare(deleteSQL)
      const result = stmt.run(keepCount)



      return result.changes
    } catch (error) {
      logger.error('[剪贴板数据库] 删除旧记录失败:', error)
      return 0
    }
  }

  /**
   * 获取记录总数
   */
  public getRecordCount(): number {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const countSQL = 'SELECT COUNT(*) as count FROM clipboard_history'
      const stmt = this.db.prepare(countSQL)
      const result = stmt.get() as { count: number }

      return result.count
    } catch (error) {
      logger.error('[剪贴板数据库] 获取记录总数失败:', error)
      return 0
    }
  }

  /**
   * 执行事务操作
   */
  public transaction<T>(fn: () => T): T {
    if (!this.db) throw new Error('数据库未初始化')

    const transaction = this.db.transaction(fn)
    return transaction()
  }

  /**
   * 关闭数据库连接
   */
  public close(): void {
    if (this.db) {
      try {
        this.db.close()
        this.db = null
        // 清空应用信息缓存
        this.clearAppCache()

      } catch (error) {
        logger.error('[剪贴板数据库] 关闭数据库连接失败:', error)
      }
    }
  }

  /**
   * 检查数据库连接状态
   */
  public isConnected(): boolean {
    return this.db !== null && this.db.open
  }

  /**
   * 获取数据库文件路径
   */
  public getDatabasePath(): string {
    return this.dbPath
  }

  /**
   * 删除指定时间戳之前的记录（排除有标签的记录）
   */
  public deleteRecordsByTimestamp(beforeTimestamp: number): number {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      // 删除过期且没有标签的记录
      const deleteSQL = `
        DELETE FROM clipboard_history
        WHERE timestamp < ?
        AND content_id NOT IN (
          SELECT DISTINCT content_id FROM clipboard_history_tags
        )
      `
      const stmt = this.db.prepare(deleteSQL)
      const result = stmt.run(beforeTimestamp)



      return result.changes
    } catch (error) {
      logger.error('[剪贴板数据库] 按时间戳删除记录失败:', error)
      return 0
    }
  }

  /**
   * 获取指定时间戳之前的记录数量（排除有标签的记录）
   */
  public countRecordsByTimestamp(beforeTimestamp: number): number {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const countSQL = `
        SELECT COUNT(*) as count
        FROM clipboard_history
        WHERE timestamp < ?
        AND content_id NOT IN (
          SELECT DISTINCT content_id FROM clipboard_history_tags
        )
      `
      const stmt = this.db.prepare(countSQL)
      const result = stmt.get(beforeTimestamp) as { count: number }

      return result.count
    } catch (error) {
      logger.error('[剪贴板数据库] 按时间戳统计记录失败:', error)
      return 0
    }
  }

  /**
   * 获取指定时间戳之前的过期记录ID列表（排除有标签的记录）
   */
  public getExpiredRecordIds(beforeTimestamp: number): string[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const querySQL = `
        SELECT content_id
        FROM clipboard_history
        WHERE timestamp < ?
        AND content_id NOT IN (
          SELECT DISTINCT content_id FROM clipboard_history_tags
        )
      `
      const stmt = this.db.prepare(querySQL)
      const results = stmt.all(beforeTimestamp) as Array<{ content_id: string }>

      return results.map(result => result.content_id)
    } catch (error) {
      logger.error('[剪贴板数据库] 获取过期记录ID失败:', error)
      return []
    }
  }

  /**
   * 获取数据库实例（用于高级操作，如事务）
   */
  public getDatabase(): Database.Database | null {
    return this.db
  }

  // ==================== 标签组管理方法 ====================

  /**
   * 创建新的标签组
   */
  public createTagGroup(tag: TagGroup): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const insertSQL = `
        INSERT INTO tag_groups (
          id, name, color, description, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?)
      `

      const stmt = this.db.prepare(insertSQL)
      const result = stmt.run(
        tag.id,
        tag.name,
        tag.color,
        tag.description || null,
        tag.createdAt,
        tag.updatedAt
      )

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 创建标签组失败:', error)
      return false
    }
  }

  /**
   * 获取所有标签组
   */
  public getAllTagGroups(options?: TagQueryOptions): TagGroupRecord[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      let sql = 'SELECT * FROM tag_groups'
      const params: any[] = []

      // 构建ORDER BY子句
      if (options?.orderBy) {
        sql += ` ORDER BY ${options.orderBy} ${options.orderDirection || 'ASC'}`
      } else {
        sql += ' ORDER BY created_at DESC'
      }

      // 添加LIMIT和OFFSET
      if (options?.limit) {
        sql += ' LIMIT ?'
        params.push(options.limit)

        if (options?.offset) {
          sql += ' OFFSET ?'
          params.push(options.offset)
        }
      }

      const stmt = this.db.prepare(sql)
      const records = stmt.all(...params) as TagGroupRecord[]

      return records
    } catch (error) {
      logger.error('[剪贴板数据库] 获取标签组列表失败:', error)
      return []
    }
  }

  /**
   * 根据ID获取标签组
   */
  public getTagGroupById(tagId: string): TagGroupRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = 'SELECT * FROM tag_groups WHERE id = ?'
      const stmt = this.db.prepare(selectSQL)
      const record = stmt.get(tagId) as TagGroupRecord | undefined

      return record || null
    } catch (error) {
      logger.error('[剪贴板数据库] 获取标签组失败:', error)
      return null
    }
  }

  /**
   * 根据名称获取标签组
   */
  public getTagGroupByName(name: string): TagGroupRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = 'SELECT * FROM tag_groups WHERE name = ?'
      const stmt = this.db.prepare(selectSQL)
      const record = stmt.get(name) as TagGroupRecord | undefined

      return record || null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据名称获取标签组失败:', error)
      return null
    }
  }

  /**
   * 更新标签组
   */
  public updateTagGroup(tagId: string, updates: Partial<TagGroup>): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const updateFields: string[] = []
      const params: any[] = []

      if (updates.name !== undefined) {
        updateFields.push('name = ?')
        params.push(updates.name)
      }
      if (updates.color !== undefined) {
        updateFields.push('color = ?')
        params.push(updates.color)
      }
      if (updates.description !== undefined) {
        updateFields.push('description = ?')
        params.push(updates.description)
      }
      if (updates.updatedAt !== undefined) {
        updateFields.push('updated_at = ?')
        params.push(updates.updatedAt)
      }

      if (updateFields.length === 0) {
        return false
      }

      const updateSQL = `UPDATE tag_groups SET ${updateFields.join(', ')} WHERE id = ?`
      params.push(tagId)

      const stmt = this.db.prepare(updateSQL)
      const result = stmt.run(...params)

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 更新标签组失败:', error)
      return false
    }
  }

  /**
   * 删除标签组
   */
  public deleteTagGroup(tagId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      return this.transaction(() => {
        // 先删除关联记录
        const deleteAssociationsSQL = 'DELETE FROM clipboard_history_tags WHERE tag_id = ?'
        const deleteAssociationsStmt = this.db!.prepare(deleteAssociationsSQL)
        deleteAssociationsStmt.run(tagId)

        // 再删除标签组
        const deleteTagSQL = 'DELETE FROM tag_groups WHERE id = ?'
        const deleteTagStmt = this.db!.prepare(deleteTagSQL)
        const result = deleteTagStmt.run(tagId)

        return result.changes > 0
      })
    } catch (error) {
      logger.error('[剪贴板数据库] 删除标签组失败:', error)
      return false
    }
  }

  /**
   * 获取标签组统计信息
   */
  public getTagGroupStats(): TagStats[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const statsSQL = `
        SELECT
          tg.id as tagId,
          tg.name as tagName,
          COUNT(cht.content_id) as recordCount
        FROM tag_groups tg
        LEFT JOIN clipboard_history_tags cht ON tg.id = cht.tag_id
        GROUP BY tg.id, tg.name
        ORDER BY recordCount DESC, tg.name ASC
      `

      const stmt = this.db.prepare(statsSQL)
      const stats = stmt.all() as TagStats[]

      return stats
    } catch (error) {
      logger.error('[剪贴板数据库] 获取标签组统计信息失败:', error)
      return []
    }
  }

  // ==================== 历史记录标签绑定方法 ====================

  /**
   * 为历史记录绑定标签
   */
  public bindTagToRecord(contentId: string, tagId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const insertSQL = `
        INSERT OR IGNORE INTO clipboard_history_tags (
          content_id, tag_id, created_at
        ) VALUES (?, ?, ?)
      `

      const stmt = this.db.prepare(insertSQL)
      const result = stmt.run(contentId, tagId, Date.now())

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 绑定标签失败:', error)
      return false
    }
  }

  /**
   * 为历史记录批量绑定标签
   */
  public bindTagsToRecord(contentId: string, tagIds: string[]): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      return this.transaction(() => {
        const insertSQL = `
          INSERT OR IGNORE INTO clipboard_history_tags (
            content_id, tag_id, created_at
          ) VALUES (?, ?, ?)
        `
        const stmt = this.db!.prepare(insertSQL)
        const now = Date.now()

        let successCount = 0
        for (const tagId of tagIds) {
          const result = stmt.run(contentId, tagId, now)
          if (result.changes > 0) {
            successCount++
          }
        }

        return successCount > 0
      })
    } catch (error) {
      logger.error('[剪贴板数据库] 批量绑定标签失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的标签绑定
   */
  public unbindTagFromRecord(contentId: string, tagId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const deleteSQL = 'DELETE FROM clipboard_history_tags WHERE content_id = ? AND tag_id = ?'
      const stmt = this.db.prepare(deleteSQL)
      const result = stmt.run(contentId, tagId)

      // 如果这是最后一个标签，更新历史记录的时间戳
      if (result.changes > 0) {
        this.updateRecordTimestampIfNoTags(contentId)
      }

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 解除标签绑定失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的所有标签绑定
   */
  public unbindAllTagsFromRecord(contentId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const deleteSQL = 'DELETE FROM clipboard_history_tags WHERE content_id = ?'
      const stmt = this.db.prepare(deleteSQL)
      const result = stmt.run(contentId)

      // 更新历史记录的时间戳
      if (result.changes > 0) {
        this.updateRecordTimestampIfNoTags(contentId)
      }

      return result.changes > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 解除所有标签绑定失败:', error)
      return false
    }
  }

  /**
   * 获取历史记录绑定的标签
   */
  public getRecordTags(contentId: string): TagGroupRecord[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = `
        SELECT tg.*
        FROM tag_groups tg
        INNER JOIN clipboard_history_tags cht ON tg.id = cht.tag_id
        WHERE cht.content_id = ?
        ORDER BY cht.created_at ASC
      `

      const stmt = this.db.prepare(selectSQL)
      const tags = stmt.all(contentId) as TagGroupRecord[]

      return tags
    } catch (error) {
      logger.error('[剪贴板数据库] 获取记录标签失败:', error)
      return []
    }
  }

  /**
   * 批量获取多个历史记录的标签
   */
  private getBatchRecordTags(contentIds: string[]): Map<string, TagGroupRecord[]> {
    if (!this.db) throw new Error('数据库未初始化')

    const tagsMap = new Map<string, TagGroupRecord[]>()

    if (contentIds.length === 0) {
      return tagsMap
    }

    try {
      // 构建 IN 查询的占位符
      const placeholders = contentIds.map(() => '?').join(',')
      const selectSQL = `
        SELECT tg.*, cht.content_id, cht.created_at as tag_created_at
        FROM tag_groups tg
        INNER JOIN clipboard_history_tags cht ON tg.id = cht.tag_id
        WHERE cht.content_id IN (${placeholders})
        ORDER BY cht.content_id, cht.created_at ASC
      `

      const stmt = this.db.prepare(selectSQL)
      const results = stmt.all(...contentIds) as Array<TagGroupRecord & { content_id: string, tag_created_at: string }>

      // 按 content_id 分组
      for (const result of results) {
        const { content_id, tag_created_at, ...tagData } = result

        if (!tagsMap.has(content_id)) {
          tagsMap.set(content_id, [])
        }

        tagsMap.get(content_id)!.push(tagData)
      }

      return tagsMap
    } catch (error) {
      logger.error('[剪贴板数据库] 批量获取记录标签失败:', error)
      return tagsMap
    }
  }

  /**
   * 检查历史记录是否有标签
   */
  public hasRecordTags(contentId: string): boolean {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const countSQL = 'SELECT COUNT(*) as count FROM clipboard_history_tags WHERE content_id = ?'
      const stmt = this.db.prepare(countSQL)
      const result = stmt.get(contentId) as { count: number }

      return result.count > 0
    } catch (error) {
      logger.error('[剪贴板数据库] 检查记录标签失败:', error)
      return false
    }
  }

  /**
   * 如果记录没有标签，更新其时间戳为当前时间
   */
  private updateRecordTimestampIfNoTags(contentId: string): void {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const hasTagsSQL = 'SELECT COUNT(*) as count FROM clipboard_history_tags WHERE content_id = ?'
      const hasTagsStmt = this.db.prepare(hasTagsSQL)
      const result = hasTagsStmt.get(contentId) as { count: number }

      if (result.count === 0) {
        // 没有标签了，更新时间戳
        const updateSQL = 'UPDATE clipboard_history SET timestamp = ?, last_used = ? WHERE content_id = ?'
        const updateStmt = this.db.prepare(updateSQL)
        const now = Date.now()
        updateStmt.run(now, now, contentId)
      }
    } catch (error) {
      logger.error('[剪贴板数据库] 更新记录时间戳失败:', error)
    }
  }

  /**
   * 查询带标签信息的历史记录
   */
  public queryRecordsWithTags(options: QueryOptions = {}): Array<ClipboardHistoryRecord & { tags: TagGroupRecord[], appInfo?: AppInfoRecord }> {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      // 先查询历史记录
      const records = this.queryRecords(options)

      if (records.length === 0) {
        return []
      }

      // 批量查询所有记录的标签
      const contentIds = records.map(record => record.content_id)
      const tagsMap = this.getBatchRecordTags(contentIds)

      // 批量查询应用信息
      const appIds = [...new Set(records.map(record => record.source_app).filter(Boolean))] as string[]
      const appsMap = this.getBatchApps(appIds)

      // 为每个记录分配对应的标签和应用信息
      const recordsWithTags = records.map(record => {
        return {
          ...record,
          tags: tagsMap.get(record.content_id) || [],
          appInfo: record.source_app ? appsMap.get(record.source_app) : undefined
        }
      })

      return recordsWithTags
    } catch (error) {
      logger.error('[剪贴板数据库] 查询带标签的记录失败:', error)
      return []
    }
  }

  /**
   * 获取有标签的历史记录ID列表（用于清理保护）
   */
  public getTaggedRecordIds(): string[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const querySQL = 'SELECT DISTINCT content_id FROM clipboard_history_tags'
      const stmt = this.db.prepare(querySQL)
      const results = stmt.all() as Array<{ content_id: string }>

      return results.map(result => result.content_id)
    } catch (error) {
      logger.error('[剪贴板数据库] 获取有标签的记录ID失败:', error)
      return []
    }
  }

  // ==================== 应用信息缓存管理方法 ====================

  /**
   * 生成应用缓存键（application + path）
   */
  private generateAppCacheKey(application: string, path: string): string {
    return `${application}|||${path}`
  }

  /**
   * 清理应用信息缓存（仅基于容量）
   */
  private cleanupAppCache(): void {
    // 如果缓存超过最大大小，清理最旧的一半
    if (this.appCache.size > this.appCacheMaxSize) {
      const entries = Array.from(this.appCache.entries())
      // 按更新时间排序，删除最旧的一半
      entries.sort((a, b) => a[1].updated_at - b[1].updated_at)
      const toDelete = entries.slice(0, Math.floor(entries.length / 2))

      for (const [key, app] of toDelete) {
        this.appCache.delete(key)
        this.appCacheByAppPath.delete(this.generateAppCacheKey(app.application, app.path))
      }


    }
  }

  /**
   * 将应用信息添加到缓存
   */
  private addToAppCache(app: AppInfoRecord): void {
    this.cleanupAppCache()

    const cacheKey = this.generateAppCacheKey(app.application, app.path)
    this.appCache.set(app.id, app)
    this.appCacheByAppPath.set(cacheKey, app)
  }

  /**
   * 从缓存中获取应用信息（通过ID）
   */
  private getFromAppCacheById(appId: string): AppInfoRecord | undefined {
    return this.appCache.get(appId)
  }

  /**
   * 从缓存中获取应用信息（通过应用名称和路径）
   */
  private getFromAppCacheByAppPath(application: string, path: string): AppInfoRecord | undefined {
    const cacheKey = this.generateAppCacheKey(application, path)
    return this.appCacheByAppPath.get(cacheKey)
  }

  /**
   * 从缓存中移除应用信息
   */
  private removeFromAppCache(app: AppInfoRecord): void {
    const cacheKey = this.generateAppCacheKey(app.application, app.path)
    this.appCache.delete(app.id)
    this.appCacheByAppPath.delete(cacheKey)
  }

  /**
   * 清空应用信息缓存
   */
  private clearAppCache(): void {
    this.appCache.clear()
    this.appCacheByAppPath.clear()
  }

  /**
   * 预加载常用应用信息到缓存
   */
  private preloadAppCache(): void {
    if (!this.db) return

    try {
      // 加载最近更新的应用信息到缓存
      const selectSQL = 'SELECT * FROM apps ORDER BY updated_at DESC LIMIT 100'
      const stmt = this.db.prepare(selectSQL)
      const apps = stmt.all() as AppInfoRecord[]

      for (const app of apps) {
        this.addToAppCache(app)
      }


    } catch (error) {
      logger.error('[剪贴板数据库] 预加载应用缓存失败:', error)
    }
  }

  /**
   * 获取应用缓存统计信息
   */
  public getAppCacheStats(): {
    cacheSize: number
    maxCacheSize: number
  } {
    return {
      cacheSize: this.appCache.size,
      maxCacheSize: this.appCacheMaxSize
    }
  }

  // ==================== 应用信息管理方法 ====================

  /**
   * 插入或更新应用信息
   */
  public insertOrUpdateApp(application: string, path: string, icon?: string): string | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      // 先从缓存查询是否已存在
      let existingApp = this.getFromAppCacheByAppPath(application, path)

      // 如果缓存中没有，从数据库查询
      if (!existingApp) {
        const dbApp = this.getAppByApplicationAndPathFromDB(application, path)
        if (dbApp) {
          existingApp = dbApp
          this.addToAppCache(existingApp)
        }
      }

      if (existingApp) {
        // 如果存在，更新图标和更新时间
        if (icon && icon !== existingApp.icon) {
          const now = Date.now()
          const updateSQL = `
            UPDATE apps
            SET icon = ?, updated_at = ?
            WHERE id = ?
          `
          const stmt = this.db.prepare(updateSQL)
          stmt.run(icon, now, existingApp.id)

          // 更新缓存
          const updatedApp = { ...existingApp, icon, updated_at: now }
          this.removeFromAppCache(existingApp)
          this.addToAppCache(updatedApp)
        }
        return existingApp.id
      } else {
        // 如果不存在，插入新记录
        const appId = `app_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
        const now = Date.now()

        const insertSQL = `
          INSERT INTO apps (
            id, application, path, icon, created_at, updated_at
          ) VALUES (?, ?, ?, ?, ?, ?)
        `

        const stmt = this.db.prepare(insertSQL)
        const result = stmt.run(appId, application, path, icon || null, now, now)

        if (result.changes > 0) {
          // 添加到缓存
          const newApp: AppInfoRecord = {
            id: appId,
            application,
            path,
            icon,
            created_at: now,
            updated_at: now
          }
          this.addToAppCache(newApp)
          return appId
        }

        return null
      }
    } catch (error) {
      logger.error('[剪贴板数据库] 插入或更新应用信息失败:', error)
      return null
    }
  }

  /**
   * 根据应用名称和路径从数据库获取应用信息（不使用缓存）
   */
  private getAppByApplicationAndPathFromDB(application: string, path: string): AppInfoRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = 'SELECT * FROM apps WHERE application = ? AND path = ?'
      const stmt = this.db.prepare(selectSQL)
      return stmt.get(application, path) as AppInfoRecord | null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据应用名称和路径获取应用信息失败:', error)
      return null
    }
  }

  /**
   * 根据应用名称和路径获取应用信息（使用缓存）
   */
  public getAppByApplicationAndPath(application: string, path: string): AppInfoRecord | null {
    try {
      // 先从缓存查询
      let app = this.getFromAppCacheByAppPath(application, path)

      // 如果缓存中没有，从数据库查询
      if (!app) {
        const dbApp = this.getAppByApplicationAndPathFromDB(application, path)
        if (dbApp) {
          app = dbApp
          this.addToAppCache(app)
        }
      }

      return app || null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据应用名称和路径获取应用信息失败:', error)
      return null
    }
  }

  /**
   * 根据ID从数据库获取应用信息（不使用缓存）
   */
  private getAppByIdFromDB(appId: string): AppInfoRecord | null {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = 'SELECT * FROM apps WHERE id = ?'
      const stmt = this.db.prepare(selectSQL)
      return stmt.get(appId) as AppInfoRecord | null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据ID获取应用信息失败:', error)
      return null
    }
  }

  /**
   * 根据ID获取应用信息（使用缓存）
   */
  public getAppById(appId: string): AppInfoRecord | null {
    try {
      // 先从缓存查询
      let app = this.getFromAppCacheById(appId)

      // 如果缓存中没有，从数据库查询
      if (!app) {
        const dbApp = this.getAppByIdFromDB(appId)
        if (dbApp) {
          app = dbApp
          this.addToAppCache(app)
        }
      }

      return app || null
    } catch (error) {
      logger.error('[剪贴板数据库] 根据ID获取应用信息失败:', error)
      return null
    }
  }

  /**
   * 获取所有应用信息
   */
  public getAllApps(): AppInfoRecord[] {
    if (!this.db) throw new Error('数据库未初始化')

    try {
      const selectSQL = 'SELECT * FROM apps ORDER BY updated_at DESC'
      const stmt = this.db.prepare(selectSQL)
      return stmt.all() as AppInfoRecord[]
    } catch (error) {
      logger.error('[剪贴板数据库] 获取所有应用信息失败:', error)
      return []
    }
  }

  /**
   * 批量获取应用信息（使用缓存）
   */
  private getBatchApps(appIds: string[]): Map<string, AppInfoRecord> {
    const appsMap = new Map<string, AppInfoRecord>()

    if (appIds.length === 0) {
      return appsMap
    }

    try {
      const missingIds: string[] = []

      // 先从缓存中获取
      for (const appId of appIds) {
        const cachedApp = this.getFromAppCacheById(appId)
        if (cachedApp) {
          appsMap.set(appId, cachedApp)
        } else {
          missingIds.push(appId)
        }
      }

      // 如果有缓存中没有的，从数据库批量查询
      if (missingIds.length > 0) {
        if (!this.db) throw new Error('数据库未初始化')

        const placeholders = missingIds.map(() => '?').join(',')
        const selectSQL = `SELECT * FROM apps WHERE id IN (${placeholders})`

        const stmt = this.db.prepare(selectSQL)
        const results = stmt.all(...missingIds) as AppInfoRecord[]

        // 将查询结果添加到返回映射和缓存中
        for (const result of results) {
          appsMap.set(result.id, result)
          this.addToAppCache(result)
        }
      }

      return appsMap
    } catch (error) {
      logger.error('[剪贴板数据库] 批量获取应用信息失败:', error)
      return appsMap
    }
  }
}

// 导出单例实例
export default ClipboardDatabase.getInstance()
export { ClipboardDatabase }
