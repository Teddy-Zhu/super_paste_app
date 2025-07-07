import { clipboard, BrowserWindow } from 'electron'
import clipboardDatabase from './clipboard-database'
import clipboardHistoryCleaner from './clipboard-history-cleaner'
import store from './store'
import { STORE_KEY } from './common'
import type {
  ClipboardContent,
  ClipboardItem,
  ClipboardManagerConfig,
  TagGroup,
  TagStats,
  AppInfo
} from './types/clipboard-types'
import { ClipboardDataConverter } from './types/clipboard-types'
import ActiveWindow from '@paymoapp/active-window'
//@ts-ignore
// export const ActiveWindow = __ActiveWindow.default || __ActiveWindow
/**
 * 剪贴板管理器
 * 实现 LRU 策略的剪贴板历史记录管理
 */
class ClipboardManager {
  private static instance: ClipboardManager
  private config: ClipboardManagerConfig
  private isListening: boolean = false
  private checkTimer: NodeJS.Timeout | null = null
  private lastClipboardHash: string = ''
  private historyItems: ClipboardItem[] = []

  private constructor() {
    // 从store中获取maxHistorySize配置，如果不存在则使用默认值100
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)

    this.config = {
      maxHistorySize: maxHistorySize,
      checkInterval: 200,
      enableAutoSave: false
    }

    // 初始化 ActiveWindow
    try {
      
      ActiveWindow.initialize()
      // 在 macOS 上请求屏幕录制权限
      if (process.platform === 'darwin') {
        const hasPermission = ActiveWindow.requestPermissions()
        if (!hasPermission) {
          console.warn('[剪贴板管理器] macOS 需要屏幕录制权限才能获取窗口标题')
        }
      }
      console.log('[剪贴板管理器] ActiveWindow 已初始化')
    } catch (error) {
      console.error('[剪贴板管理器] ActiveWindow 初始化失败:', error)
    }
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): ClipboardManager {
    if (!ClipboardManager.instance) {
      ClipboardManager.instance = new ClipboardManager()
    }
    return ClipboardManager.instance
  }

  /**
   * 初始化剪贴板管理器
   */
  public initialize(config?: Partial<ClipboardManagerConfig>): void {
    if (config) {
      this.config = { ...this.config, ...config }
    }

    try {
      // 初始化数据库
      clipboardDatabase.initialize()

      // 从数据库加载历史记录
      this.loadHistoryFromDatabase()

      // 设置清理器的内存数据同步回调
      clipboardHistoryCleaner.setMemoryDataSyncCallback((deletedIds: string[]) => {
        this.syncDeletedRecords(deletedIds)
      })

      // 启动历史记录清理器
      clipboardHistoryCleaner.startAutoCleanup()

      // 初始化时获取当前剪贴板内容哈希
      this.lastClipboardHash = this.generateClipboardHash()

      console.log('[剪贴板管理器] 已初始化，配置:', this.config)
      console.log('[剪贴板管理器] 从数据库加载了', this.historyItems.length, '条历史记录')
    } catch (error) {
      console.error('[剪贴板管理器] 初始化失败:', error)
      // 即使数据库初始化失败，也要继续初始化剪贴板管理器
      this.lastClipboardHash = this.generateClipboardHash()
    }
  }

  /**
   * 开始监听剪贴板变化
   */
  public startListening(): void {
    if (this.isListening) {
      console.log('[剪贴板管理器] 已在监听中')
      return
    }

    this.isListening = true
    this.checkTimer = setInterval(() => {
      this.checkClipboardChange()
    }, this.config.checkInterval)

    console.log('[剪贴板管理器] 开始监听剪贴板变化')
  }

  /**
   * 停止监听剪贴板变化
   */
  public stopListening(): void {
    if (!this.isListening) {
      console.log('[剪贴板管理器] 未在监听中')
      return
    }

    this.isListening = false
    if (this.checkTimer) {
      clearInterval(this.checkTimer)
      this.checkTimer = null
    }

    console.log('[剪贴板管理器] 停止监听剪贴板变化')
  }

  /**
   * 检查剪贴板内容变化
   */
  private checkClipboardChange(): void {
    try {
      const currentHash = this.generateClipboardHash()

      if (currentHash && currentHash !== this.lastClipboardHash) {
        this.lastClipboardHash = currentHash

        // 获取当前激活窗口信息并打印
        // this.logActiveWindowInfo()

        if (this.config.enableAutoSave) {
          const clipboardContent = this.getCurrentClipboardContent()
          this.saveClipboardItem(clipboardContent)
        }
      }
    } catch (error) {
      console.error('[剪贴板管理器] 检查剪贴板变化失败:', error)
    }
  }

  /**
   * 获取当前剪贴板所有格式的内容
   */
  private getCurrentClipboardContent(): ClipboardContent {
    const content: ClipboardContent = {}

    try {
      // 获取文本内容
      const text = clipboard.readText()
      if (text) content.text = text

      // 获取HTML内容
      const html = clipboard.readHTML()
      if (html) content.html = html

      // 获取RTF内容
      const rtf = clipboard.readRTF()
      if (rtf) content.rtf = rtf

      // 获取书签内容
      const bookmark = clipboard.readBookmark()
      if (bookmark && bookmark.title) {
        content.bookmark = JSON.stringify(bookmark)
      }

      // 获取图片内容（转为base64）
      const image = clipboard.readImage()
      if (!image.isEmpty()) {
        content.image = image.toDataURL()
      }
    } catch (error) {
      console.error('[剪贴板管理器] 读取剪贴板内容失败:', error)
    }

    return content
  }

  /**
   * 生成剪贴板内容的哈希值用于比较
   */
  private generateClipboardHash(): string {
    try {
      const content = this.getCurrentClipboardContent()
      return ClipboardDataConverter.generateContentHash(content)
    } catch (error) {
      console.error('[剪贴板管理器] 生成剪贴板哈希失败:', error)
      return ''
    }
  }

  /**
   * 保存剪贴板项到历史记录
   */
  private saveClipboardItem(content: ClipboardContent): void {
    if (!content || ClipboardDataConverter.isEmptyContent(content)) {
      return
    }

    try {
      const now = Date.now()
      const contentHash = ClipboardDataConverter.generateContentHash(content)

      // 检查是否已存在相同内容
      const existingIndex = this.historyItems.findIndex(item =>
        ClipboardDataConverter.generateContentHash(item.content) === contentHash
      )

      if (existingIndex !== -1) {
        // 更新现有项的最后使用时间并移到最前面
        const existingItem = this.historyItems[existingIndex]
        existingItem.lastUsed = now
        this.historyItems.splice(existingIndex, 1)
        this.historyItems.unshift(existingItem)

        // 同步更新数据库中的最后使用时间
        this.syncToDatabase(existingItem, true)
      } else {
        // 获取当前激活窗口的应用信息
        const sourceAppInfo = this.getCurrentAppInfo()

        // 创建新的剪贴板项
        const availableFormats = ClipboardDataConverter.getAvailableFormats(content)
        const newItem: ClipboardItem = {
          id: ClipboardDataConverter.generateId(),
          content,
          primaryType: ClipboardDataConverter.detectPrimaryType(content, availableFormats),
          availableFormats: availableFormats,
          timestamp: now,
          lastUsed: now,
          parseType: ClipboardDataConverter.detectParseType(content, availableFormats),
          tags: [],
          sourceAppInfo: sourceAppInfo
        }

        // 添加到历史记录开头
        this.historyItems.unshift(newItem)

        // 同步保存到数据库
        this.syncToDatabase(newItem, false)

        // 如果超过最大数量，删除最久未使用的项
        if (this.historyItems.length > this.config.maxHistorySize) {
          const removedItems = this.historyItems.splice(this.config.maxHistorySize)
          // 从数据库中删除被移除的项
          removedItems.forEach(item => {
            this.removeFromDatabase(item.id)
          })
        }
      }

      console.log('[剪贴板管理器] 已保存剪贴板项，当前历史记录数量:', this.historyItems.length)

      // 通知前端剪贴板历史已更新
      this.notifyFrontendUpdate()
    } catch (error) {
      console.error('[剪贴板管理器] 保存剪贴板项失败:', error)
    }
  }

  /**
   * 通知前端剪贴板历史更新
   */
  private notifyFrontendUpdate(): void {
    try {
      const allWindows = BrowserWindow.getAllWindows()

      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('clipboard-history-updated', {
            timestamp: Date.now()
          })
        }
      })

      console.log('[剪贴板管理器] 已通知前端剪贴板历史更新')
    } catch (error) {
      console.error('[剪贴板管理器] 通知前端更新失败:', error)
    }
  }



  /**
   * 从数据库加载历史记录
   */
  private loadHistoryFromDatabase(): void {
    try {
      if (!clipboardDatabase.isConnected()) {
        console.warn('[剪贴板管理器] 数据库未连接，跳过加载历史记录')
        return
      }

      const recordsWithTags = clipboardDatabase.queryRecordsWithTags({
        limit: this.config.maxHistorySize,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      this.historyItems = recordsWithTags.map(recordWithTags => {
        const tags = recordWithTags.tags.map(tagRecord =>
          ClipboardDataConverter.tagRecordToTag(tagRecord)
        )
        const sourceAppInfo = recordWithTags.appInfo ?
          ClipboardDataConverter.appRecordToAppInfo(recordWithTags.appInfo) :
          undefined
        return ClipboardDataConverter.recordToItem(recordWithTags, tags, sourceAppInfo)
      })

      console.log('[剪贴板管理器] 从数据库加载了', this.historyItems.length, '条历史记录')
    } catch (error) {
      console.error('[剪贴板管理器] 从数据库加载历史记录失败:', error)
      this.historyItems = []
    }
  }

  /**
   * 同步剪贴板项到数据库
   */
  private syncToDatabase(item: ClipboardItem, isUpdate: boolean): void {
    try {
      if (!clipboardDatabase.isConnected()) {
        return
      }

      if (isUpdate) {
        // 更新最后使用时间
        clipboardDatabase.updateLastUsed(item.id, item.lastUsed)
      } else {
        // 如果有应用信息，先保存应用信息到数据库
        let sourceAppId: string | undefined = undefined
        if (item.sourceAppInfo) {
          sourceAppId = clipboardDatabase.insertOrUpdateApp(
            item.sourceAppInfo.application,
            item.sourceAppInfo.path,
            item.sourceAppInfo.icon
          ) || undefined
        }

        // 插入新记录
        clipboardDatabase.insertRecord(
          item.id,
          item.content,
          item.primaryType,
          item.availableFormats,
          item.timestamp,
          item.lastUsed,
          item.parseType,
          sourceAppId
        )
      }
    } catch (error) {
      console.error('[剪贴板管理器] 同步到数据库失败:', error)
    }
  }

  /**
   * 从数据库中删除记录
   */
  private removeFromDatabase(contentId: string): void {
    try {
      if (!clipboardDatabase.isConnected()) {
        return
      }

      clipboardDatabase.deleteRecord(contentId)
    } catch (error) {
      console.error('[剪贴板管理器] 从数据库删除记录失败:', error)
    }
  }

  /**
   * 同步删除内存中的记录（由清理器调用）
   */
  private syncDeletedRecords(deletedIds: string[]): void {
    try {
      const originalLength = this.historyItems.length

      // 从内存中删除指定ID的记录
      this.historyItems = this.historyItems.filter(item => !deletedIds.includes(item.id))

      const deletedCount = originalLength - this.historyItems.length

      if (deletedCount > 0) {
        console.log(`[剪贴板管理器] 已从内存中同步删除 ${deletedCount} 条过期记录`)
      }
    } catch (error) {
      console.error('[剪贴板管理器] 同步删除内存记录失败:', error)
    }
  }

  /**
   * 获取历史记录
   * @param limit 可选参数，限制返回的记录数量，如果不传则返回所有记录
   */
  public getHistory(limit?: number): ClipboardItem[] {
    if (limit && limit > 0) {
      return [...this.historyItems.slice(0, limit)]
    }
    return [...this.historyItems]
  }

  /**
   * 还原指定的剪贴板内容到当前剪贴板
   */
  public restoreClipboard(itemId: string): boolean {
    console.log('[剪贴板管理器] 开始还原剪贴板内容:', itemId)

    try {
      const item = this.historyItems.find(h => h.id === itemId)

      if (!item) {
        console.error('[剪贴板管理器] 未找到指定的剪贴板项:', itemId)
        return false
      }

      console.log('[剪贴板管理器] 找到要还原的项:')

      // 使用同步方法还原
      console.log('[剪贴板管理器] 尝试使用同步方法还原')
      const success = this.restoreClipboardSync(itemId)

      if (success) {
        console.log('[剪贴板管理器] 已还原剪贴板内容:', itemId)
        return true
      } else {
        console.error('[剪贴板管理器] 还原失败:', itemId)
        return false
      }
    } catch (error) {
      console.error('[剪贴板管理器] 还原剪贴板失败:', error)
      return false
    }
  }

  /**
   * 使用同步方式还原剪贴板内容（备用方法）
   */
  public restoreClipboardSync(itemId: string): boolean {
    console.log('[剪贴板管理器] 使用同步方式还原剪贴板内容:', itemId)

    try {
      const item = this.historyItems.find(h => h.id === itemId)

      if (!item) {
        console.error('[剪贴板管理器] 未找到指定的剪贴板项:', itemId)
        return false
      }

      // 暂停监听
      const wasListening = this.isListening
      if (wasListening) {
        this.stopListening()
      }

      try {
        const content = item.content

        // 使用 Electron 的 clipboard.write 方法一次性设置所有格式
        const clipboardData: any = {}

        if (content.text) {
          clipboardData.text = content.text
        }

        if (content.html) {
          clipboardData.html = content.html
        }

        if (content.rtf) {
          clipboardData.rtf = content.rtf
        }

        if (content.image) {
          try {
            const nativeImage = require('electron').nativeImage
            const image = nativeImage.createFromDataURL(content.image)
            if (!image.isEmpty()) {
              clipboardData.image = image
            }
          } catch (e) {
            console.warn('[剪贴板管理器] 处理图片数据失败:', e)
          }
        }

        // 一次性写入所有格式
        if (Object.keys(clipboardData).length > 0) {
          clipboard.write(clipboardData)
          console.log('[剪贴板管理器] 已使用 write 方法设置剪贴板，格式:', Object.keys(clipboardData))
        }

        // 单独处理书签（因为 write 方法不支持书签）
        if (content.bookmark) {
          try {
            const bookmark = JSON.parse(content.bookmark)
            clipboard.writeBookmark(bookmark.title, bookmark.url)
            console.log('[剪贴板管理器] 已设置书签:', bookmark.title)
          } catch (e) {
            console.warn('[剪贴板管理器] 设置书签失败:', e)
          }
        }

        // 更新最后使用时间并重新排序
        item.lastUsed = Date.now()
        const itemIndex = this.historyItems.findIndex(h => h.id === itemId)
        if (itemIndex > 0) {
          this.historyItems.splice(itemIndex, 1)
          this.historyItems.unshift(item)
        }

        // 同步更新数据库中的最后使用时间
        this.syncToDatabase(item, true)

        // 更新哈希值
        this.lastClipboardHash = this.generateClipboardHash()

        console.log('[剪贴板管理器] 同步还原完成')
        return true
      } finally {
        // 恢复监听
        if (wasListening) {
          setTimeout(() => {
            this.startListening()
          }, 100)
        }
      }
    } catch (error) {
      console.error('[剪贴板管理器] 同步还原失败:', error)
      return false
    }
  }

  /**
   * 手动设置剪贴板内容（不触发监控）
   * @param content 要设置的剪贴板内容
   * @param updateHash 是否更新内部哈希值，默认为true
   */
  public setClipboardContent(content: ClipboardContent, updateHash: boolean = true): boolean {
    try {
      // 暂停监听以避免触发变更检测
      const wasListening = this.isListening
      if (wasListening) {
        this.stopListening()
      }

      try {
        let setCount = 0

        // 设置文本内容
        if (content.text) {
          try {
            clipboard.writeText(content.text)
            setCount++
          } catch (e) {
            console.error('[剪贴板管理器] 设置文本失败:', e)
          }
        }

        // 设置HTML内容
        if (content.html) {
          try {
            clipboard.writeHTML(content.html)
            setCount++
          } catch (e) {
            console.error('[剪贴板管理器] 设置HTML失败:', e)
          }
        }

        // 设置RTF内容
        if (content.rtf) {
          try {
            clipboard.writeRTF(content.rtf)
            setCount++
          } catch (e) {
            console.error('[剪贴板管理器] 设置RTF失败:', e)
          }
        }

        // 设置书签内容
        if (content.bookmark) {
          try {
            const bookmark = JSON.parse(content.bookmark)
            clipboard.writeBookmark(bookmark.title, bookmark.url)
            setCount++
          } catch (e) {
            console.error('[剪贴板管理器] 设置书签失败:', e)
          }
        }

        // 设置图片内容
        if (content.image) {
          try {
            const nativeImage = require('electron').nativeImage
            const image = nativeImage.createFromDataURL(content.image)
            if (!image.isEmpty()) {
              clipboard.writeImage(image)
              setCount++
            }
          } catch (e) {
            console.error('[剪贴板管理器] 设置图片失败:', e)
          }
        }

        // 检查是否至少设置了一种格式
        if (setCount === 0) {
          return false
        }

        // 更新内部哈希值以保持同步
        if (updateHash) {
          this.lastClipboardHash = this.generateClipboardHash()
        }

        return true
      } finally {
        // 恢复监听状态
        if (wasListening) {
          setTimeout(() => {
            this.startListening()
          }, 150)
        }
      }
    } catch (error) {
      console.error('[剪贴板管理器] 设置剪贴板内容失败:', error)
      return false
    }
  }

  /**
   * 删除指定的历史记录项
   */
  public deleteHistoryItem(itemId: string): boolean {
    try {
      const itemIndex = this.historyItems.findIndex(h => h.id === itemId)

      if (itemIndex === -1) {
        console.error('[剪贴板管理器] 未找到指定的剪贴板项:', itemId)
        return false
      }

      // 从内存中删除
      this.historyItems.splice(itemIndex, 1)

      // 从数据库中删除
      this.removeFromDatabase(itemId)

      console.log('[剪贴板管理器] 已删除剪贴板项:', itemId)
      return true
    } catch (error) {
      console.error('[剪贴板管理器] 删除剪贴板项失败:', error)
      return false
    }
  }

  /**
   * 清空所有历史记录
   */
  public clearHistory(): void {
    try {
      // 清空内存中的历史记录
      this.historyItems = []

      // 清空数据库中的历史记录
      if (clipboardDatabase.isConnected()) {
        clipboardDatabase.clearAllRecords()
      }

      console.log('[剪贴板管理器] 已清空所有历史记录')
    } catch (error) {
      console.error('[剪贴板管理器] 清空历史记录失败:', error)
    }
  }

  /**
   * 获取配置
   */
  public getConfig(): ClipboardManagerConfig {
    return { ...this.config }
  }

  /**
   * 更新配置
   */
  public updateConfig(newConfig: Partial<ClipboardManagerConfig>): void {
    this.config = { ...this.config, ...newConfig }
    console.log('[剪贴板管理器] 配置已更新:', this.config)
  }

  /**
   * 从store更新maxHistorySize配置
   */
  public updateMaxHistorySizeFromStore(): void {
    const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100)
    this.config.maxHistorySize = maxHistorySize
    console.log('[剪贴板管理器] 从store更新maxHistorySize:', maxHistorySize)

    // 如果当前历史记录数量超过新的限制，需要删除多余的记录
    if (this.historyItems.length > maxHistorySize) {
      const removedItems = this.historyItems.splice(maxHistorySize)
      // 从数据库中删除被移除的项
      removedItems.forEach(item => {
        this.removeFromDatabase(item.id)
      })
      console.log('[剪贴板管理器] 已删除', removedItems.length, '条超出限制的历史记录')
    }
  }

  /**
   * 从store更新maxKeepTime配置并更新清理器
   */
  public updateMaxKeepTimeFromStore(): void {
    clipboardHistoryCleaner.updateConfigFromStore()
    console.log('[剪贴板管理器] 已更新历史记录清理器配置')
  }

  /**
   * 获取监听状态
   */
  public isListeningActive(): boolean {
    return this.isListening
  }

  /**
   * 强制同步当前剪贴板内容的哈希值
   * 用于在外部修改剪贴板后保持内部状态同步
   */
  public syncClipboardHash(): void {
    try {
      this.lastClipboardHash = this.generateClipboardHash()
      console.log('[剪贴板管理器] 已同步剪贴板哈希值')
    } catch (error) {
      console.error('[剪贴板管理器] 同步剪贴板哈希值失败:', error)
    }
  }

  /**
   * 获取当前剪贴板内容（不触发监控）
   */
  public getCurrentClipboard(): ClipboardContent {
    return this.getCurrentClipboardContent()
  }

  /**
   * 测试剪贴板功能
   */
  public testClipboard(): void {
    try {
      const originalText = clipboard.readText()
      const testText = `测试文本 ${Date.now()}`

      clipboard.writeText(testText)
      const readText = clipboard.readText()

      if (readText === testText) {
        console.log('[剪贴板管理器] ✅ 剪贴板基本功能正常')
      } else {
        console.error('[剪贴板管理器] ❌ 剪贴板基本功能异常')
      }

      // 恢复原始内容
      if (originalText) {
        clipboard.writeText(originalText)
      }
    } catch (error) {
      console.error('[剪贴板管理器] 测试剪贴板功能失败:', error)
    }
  }

  /**
   * 验证剪贴板内容是否可用
   */
  public verifyClipboardContent(): { isValid: boolean; details: any } {
    try {
      const text = clipboard.readText()
      const html = clipboard.readHTML()
      const rtf = clipboard.readRTF()
      const image = clipboard.readImage()
      const bookmark = clipboard.readBookmark()

      const details = {
        text: {
          available: !!text,
          length: text?.length || 0,
          preview: text?.substring(0, 50) + (text?.length > 50 ? '...' : '')
        },
        html: {
          available: !!html,
          length: html?.length || 0
        },
        rtf: {
          available: !!rtf,
          length: rtf?.length || 0
        },
        image: {
          available: !image.isEmpty(),
          size: image.isEmpty() ? null : image.getSize()
        },
        bookmark: {
          available: !!(bookmark?.title),
          title: bookmark?.title || '',
          url: bookmark?.url || ''
        }
      }

      const isValid = !!(text || html || rtf || !image.isEmpty() || bookmark?.title)

      console.log('[剪贴板管理器] 剪贴板验证结果:', { isValid, details })
      return { isValid, details }
    } catch (error) {
      console.error('[剪贴板管理器] 验证剪贴板内容失败:', error)
      return { isValid: false, details: { error: error instanceof Error ? error.message : String(error) } }
    }
  }

  // ==================== 标签管理方法 ====================

  /**
   * 创建新标签组
   */
  public createTagGroup(name: string, color: string, description?: string): string | null {
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

      const success = clipboardDatabase.createTagGroup(tag)
      if (success) {
        console.log('[剪贴板管理器] 创建标签组成功:', tag)
        return tagId
      } else {
        console.error('[剪贴板管理器] 创建标签组失败')
        return null
      }
    } catch (error) {
      console.error('[剪贴板管理器] 创建标签组失败:', error)
      return null
    }
  }

  /**
   * 获取所有标签组
   */
  public getAllTagGroups(): TagGroup[] {
    try {
      const records = clipboardDatabase.getAllTagGroups()
      return records.map(record => ClipboardDataConverter.tagRecordToTag(record))
    } catch (error) {
      console.error('[剪贴板管理器] 获取标签组列表失败:', error)
      return []
    }
  }

  /**
   * 更新标签组
   */
  public updateTagGroup(tagId: string, updates: Partial<Omit<TagGroup, 'id' | 'createdAt'>>): boolean {
    try {
      const updateData = {
        ...updates,
        updatedAt: Date.now()
      }

      const success = clipboardDatabase.updateTagGroup(tagId, updateData)
      if (success) {
        console.log('[剪贴板管理器] 更新标签组成功:', tagId, updateData)
      }
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 更新标签组失败:', error)
      return false
    }
  }

  /**
   * 删除标签组
   */
  public deleteTagGroup(tagId: string): boolean {
    try {
      const success = clipboardDatabase.deleteTagGroup(tagId)
      if (success) {
        console.log('[剪贴板管理器] 删除标签组成功:', tagId)
        // 重新加载历史记录以更新标签信息
        this.loadHistoryFromDatabase()
      }
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 删除标签组失败:', error)
      return false
    }
  }

  /**
   * 为历史记录绑定标签
   */
  public bindTagToRecord(contentId: string, tagId: string): boolean {
    try {
      const success = clipboardDatabase.bindTagToRecord(contentId, tagId)
      if (success) {
        // 更新内存中的历史记录
        const item = this.historyItems.find(item => item.id === contentId)
        if (item) {
          const tagRecord = clipboardDatabase.getTagGroupById(tagId)
          if (tagRecord) {
            const tag = ClipboardDataConverter.tagRecordToTag(tagRecord)
            if (!item.tags) item.tags = []
            if (!item.tags.find(t => t.id === tagId)) {
              item.tags.push(tag)
            }
          }
        }
        console.log('[剪贴板管理器] 绑定标签成功:', contentId, tagId)
      }
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 绑定标签失败:', error)
      return false
    }
  }

  /**
   * 解除历史记录的标签绑定
   */
  public unbindTagFromRecord(contentId: string, tagId: string): boolean {
    try {
      const success = clipboardDatabase.unbindTagFromRecord(contentId, tagId)
      if (success) {
        // 更新内存中的历史记录
        const item = this.historyItems.find(item => item.id === contentId)
        if (item && item.tags) {
          item.tags = item.tags.filter(tag => tag.id !== tagId)
        }
        console.log('[剪贴板管理器] 解除标签绑定成功:', contentId, tagId)
      }
      return success
    } catch (error) {
      console.error('[剪贴板管理器] 解除标签绑定失败:', error)
      return false
    }
  }

  /**
   * 按标签筛选历史记录
   */
  public getRecordsByTags(tagIds: string[]): ClipboardItem[] {
    try {
      const recordsWithTags = clipboardDatabase.queryRecordsWithTags({
        tagIds,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return recordsWithTags.map(recordWithTags => {
        const tags = recordWithTags.tags.map(tagRecord =>
          ClipboardDataConverter.tagRecordToTag(tagRecord)
        )
        const sourceAppInfo = recordWithTags.appInfo ?
          ClipboardDataConverter.appRecordToAppInfo(recordWithTags.appInfo) :
          undefined
        return ClipboardDataConverter.recordToItem(recordWithTags, tags, sourceAppInfo)
      })
    } catch (error) {
      console.error('[剪贴板管理器] 按标签筛选记录失败:', error)
      return []
    }
  }

  /**
   * 按解析类型筛选历史记录
   */
  public getRecordsByParseType(parseType: string): ClipboardItem[] {
    try {
      const recordsWithTags = clipboardDatabase.queryRecordsWithTags({
        parseType,
        orderBy: 'last_used',
        orderDirection: 'DESC'
      })

      return recordsWithTags.map(recordWithTags => {
        const tags = recordWithTags.tags.map(tagRecord =>
          ClipboardDataConverter.tagRecordToTag(tagRecord)
        )
        const sourceAppInfo = recordWithTags.appInfo ?
          ClipboardDataConverter.appRecordToAppInfo(recordWithTags.appInfo) :
          undefined
        return ClipboardDataConverter.recordToItem(recordWithTags, tags, sourceAppInfo)
      })
    } catch (error) {
      console.error('[剪贴板管理器] 按解析类型筛选记录失败:', error)
      return []
    }
  }

  /**
   * 获取标签统计信息
   */
  public getTagStats(): TagStats[] {
    try {
      return clipboardDatabase.getTagGroupStats()
    } catch (error) {
      console.error('[剪贴板管理器] 获取标签统计信息失败:', error)
      return []
    }
  }

  /**
   * 搜索历史记录（支持标签和内容筛选）
   */
  public searchHistory(options: {
    limit?: number
    offset?: number
    tagIds?: string[]
    parseType?: string
    searchText?: string
    orderBy?: 'timestamp' | 'last_used'
    orderDirection?: 'ASC' | 'DESC'
  } = {}): ClipboardItem[] {
    try {
      const recordsWithTags = clipboardDatabase.queryRecordsWithTags({
        limit: options.limit,
        offset: options.offset,
        tagIds: options.tagIds,
        parseType: options.parseType,
        searchText: options.searchText,
        orderBy: options.orderBy || 'last_used',
        orderDirection: options.orderDirection || 'DESC'
      })

      return recordsWithTags.map(recordWithTags => {
        const tags = recordWithTags.tags.map(tagRecord =>
          ClipboardDataConverter.tagRecordToTag(tagRecord)
        )
        const sourceAppInfo = recordWithTags.appInfo ?
          ClipboardDataConverter.appRecordToAppInfo(recordWithTags.appInfo) :
          undefined
        return ClipboardDataConverter.recordToItem(recordWithTags, tags, sourceAppInfo)
      })
    } catch (error) {
      console.error('[剪贴板管理器] 搜索历史记录失败:', error)
      return []
    }
  }

  /**
   * 获取当前激活窗口的应用信息
   */
  private getCurrentAppInfo(): AppInfo | undefined {
    try {
      const activeWindow = ActiveWindow.getActiveWindow()

      if (!activeWindow.application || !activeWindow.path) {
        return undefined
      }

      return {
        id: ClipboardDataConverter.generateAppId(),
        application: activeWindow.application,
        path: activeWindow.path,
        icon: activeWindow.icon,
        createdAt: Date.now(),
        updatedAt: Date.now()
      }
    } catch (error) {
      console.error('[剪贴板管理器] 获取当前应用信息失败:', error)
      return undefined
    }
  }

}

// 导出单例实例
export default ClipboardManager.getInstance()
export { ClipboardManager }


