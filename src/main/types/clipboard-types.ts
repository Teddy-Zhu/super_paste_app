import { HashUtils } from '../utils/common'

/**
 * 剪贴板内容数据
 */
export interface ClipboardContent {
  text?: string
  html?: string
  image?: string // base64 编码的图片数据
  rtf?: string
  bookmark?: string
}

/**
 * 应用信息
 */
export interface AppInfo {
  id: string
  application: string
  path: string
  icon?: string // base64 编码的应用图标
  createdAt: number
  updatedAt: number
}

/**
 * 数据库中的应用信息记录
 */
export interface AppInfoRecord {
  id: string
  application: string
  path: string
  icon?: string
  created_at: number
  updated_at: number
}

/**
 * 剪贴板历史记录项（内存中使用）
 */
export interface ClipboardItem {
  id: string
  content: ClipboardContent
  primaryType: 'text' | 'html' | 'image' | 'rtf' | 'bookmark' | 'empty'
  availableFormats: string[]
  timestamp: number
  lastUsed: number
  parseType: string // 解析类型字段（必填）
  tags?: TagGroup[] // 绑定的标签列表
  source_app?: string // 原始的来源应用ID字段
}

/**
 * 数据库中的剪贴板历史记录项
 */
export interface ClipboardHistoryRecord {
  id: number
  content_id: string
  hash: string
  content_text?: string
  content_html?: string
  content_image?: string
  content_rtf?: string
  content_bookmark?: string
  primary_type: string
  available_formats: string
  timestamp: number
  last_used: number
  parse_type: string // 解析类型字段（必填）
  source_app?: string // 来源应用ID
}

/**
 * 剪贴板管理器配置
 */
export interface ClipboardManagerConfig {
  maxHistorySize: number
  checkInterval: number
  enableAutoSave: boolean
}

/**
 * 历史记录清理配置
 */
export interface CleanupConfig {
  maxKeepTime: number // 最大保存时间（天），-1 表示不限制
  maxHistorySize: number // 最大历史记录数量，-1 表示不限制
  cleanupInterval: number // 清理检查间隔（毫秒）
  enableAutoCleanup: boolean // 是否启用自动清理
  enableCountBasedCleanup: boolean // 是否启用基于数量的清理
}

/**
 * 清理统计信息
 */
export interface CleanupStats {
  totalRecords: number // 清理前总记录数
  expiredRecords: number // 过期记录数
  deletedRecords: number // 实际删除的记录数
  cleanupTime: number // 清理耗时（毫秒）
}

/**
 * 查询选项
 */
export interface QueryOptions {
  limit?: number
  offset?: number
  orderBy?: 'timestamp' | 'last_used'
  orderDirection?: 'ASC' | 'DESC'
  tagIds?: string[] // 按标签ID筛选
  parseType?: string // 按解析类型筛选
  searchText?: string // 内容模糊搜索
}

/**
 * 标签组
 */
export interface TagGroup {
  id: string
  name: string
  color: string
  description?: string
  createdAt: number
  updatedAt: number
}

/**
 * 数据库中的标签组记录
 */
export interface TagGroupRecord {
  id: string
  name: string
  color: string
  description?: string
  created_at: number
  updated_at: number
}

/**
 * 历史记录与标签的关联记录
 */
export interface ClipboardHistoryTagRecord {
  id: number
  content_id: string
  tag_id: string
  created_at: number
}

/**
 * 标签统计信息
 */
export interface TagStats {
  tagId: string
  tagName: string
  recordCount: number
}

/**
 * 标签查询选项
 */
export interface TagQueryOptions {
  limit?: number
  offset?: number
  orderBy?: 'name' | 'created_at' | 'updated_at'
  orderDirection?: 'ASC' | 'DESC'
  includeStats?: boolean // 是否包含统计信息
}

/**
 * 数据转换工具类
 */
export class ClipboardDataConverter {
  /**
   * 将数据库记录转换为内存中的剪贴板项
   */
  static recordToItem(record: ClipboardHistoryRecord, tags?: TagGroup[]): ClipboardItem {
    const content: ClipboardContent = {}
    if (record.content_text) content.text = record.content_text
    if (record.content_html) content.html = record.content_html
    if (record.content_image) content.image = record.content_image
    if (record.content_rtf) content.rtf = record.content_rtf
    if (record.content_bookmark) content.bookmark = record.content_bookmark

    return {
      id: record.content_id,
      content,
      primaryType: record.primary_type as ClipboardItem['primaryType'],
      availableFormats: JSON.parse(record.available_formats),
      timestamp: record.timestamp,
      lastUsed: record.last_used,
      parseType: record.parse_type,
      tags: tags || [],
      source_app: record.source_app // 包含原始的来源应用ID字段
    }
  }

  /**
   * 将内存中的剪贴板项转换为数据库记录格式（用于插入）
   */
  static itemToRecordData(item: ClipboardItem): {
    content_id: string
    content_text?: string
    content_html?: string
    content_image?: string
    content_rtf?: string
    content_bookmark?: string
    primary_type: string
    available_formats: string
    timestamp: number
    last_used: number
    parse_type: string
    source_app?: string
  } {
    return {
      content_id: item.id,
      content_text: item.content.text,
      content_html: item.content.html,
      content_image: item.content.image,
      content_rtf: item.content.rtf,
      content_bookmark: item.content.bookmark,
      primary_type: item.primaryType,
      available_formats: JSON.stringify(item.availableFormats),
      timestamp: item.timestamp,
      last_used: item.lastUsed,
      parse_type: item.parseType,
      source_app: item.source_app
    }
  }

  /**
   * 检测主要内容类型
   */
  static detectPrimaryType(content: ClipboardContent, availableFormats?: string[]): ClipboardItem['primaryType'] {
    const formats = availableFormats || ClipboardDataConverter.getAvailableFormats(content)

    if (content.image) return 'image'

    // 当 availableFormats 中存在 rtf 时，RTF 优先级高于 html
    if (formats.includes('rtf') && content.rtf) return 'rtf'

    if (content.html) return 'html'
    if (content.bookmark) return 'bookmark'
    if (content.text) return 'text'
    return 'empty'
  }

  /**
   * 获取可用格式列表
   */
  static getAvailableFormats(content: ClipboardContent): string[] {
    const formats: string[] = []
    if (content.text) formats.push('text')
    if (content.html) formats.push('html')
    if (content.image) formats.push('image')
    if (content.rtf) formats.push('rtf')
    if (content.bookmark) formats.push('bookmark')
    return formats
  }

  /**
   * 检查内容是否为空
   */
  static isEmptyContent(content: ClipboardContent): boolean {
    return !content.text && !content.html && !content.image && !content.rtf && !content.bookmark
  }

  /**
   * 生成内容哈希值用于比较
   * 优化版本：对于图片内容，使用更高效的哈希策略，避免大图片造成CPU压力
   */
  static generateContentHash(content: ClipboardContent): string {
    // 创建一个优化的内容对象用于哈希计算
    const optimizedContent: ClipboardContent = {}

    // 直接复制非图片内容
    if (content.text) optimizedContent.text = content.text
    if (content.html) optimizedContent.html = content.html
    if (content.rtf) optimizedContent.rtf = content.rtf
    if (content.bookmark) optimizedContent.bookmark = content.bookmark

    // 对图片内容进行优化处理
    if (content.image) {
      optimizedContent.image = ClipboardDataConverter.generateImageHash(content.image)
    }

    return HashUtils.generateContentHash(optimizedContent)
  }

  /**
   * 生成图片内容的轻量级哈希
   * 使用采样策略而不是完整内容，大幅减少CPU消耗
   */
  private static generateImageHash(imageData: string): string {
    if (!imageData) return ''

    const dataLength = imageData.length

    // 小图片（<= 4KB base64）直接使用完整数据
    if (dataLength <= 4096) {
      return imageData
    }

    // 大图片使用采样策略：
    // 1. 数据头部（包含格式信息）
    // 2. 数据长度
    // 3. 每隔一定间隔采样
    // 4. 数据尾部
    const head = imageData.substring(0, 512)  // 头部512字节
    const tail = imageData.substring(dataLength - 512)  // 尾部512字节

    // 中间部分采样：每隔1000字节取32字节
    let samples = ''
    const sampleInterval = 1000
    const sampleSize = 32
    for (let i = 1000; i < dataLength - 1000; i += sampleInterval) {
      samples += imageData.substring(i, Math.min(i + sampleSize, dataLength))
    }

    // 组合采样数据
    return `${head}|${dataLength}|${samples}|${tail}`
  }

  /**
   * 生成唯一ID
   */
  static generateId(): string {
    return `clipboard_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
  }

  /**
   * 生成标签ID
   */
  static generateTagId(): string {
    return `tag_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
  }

  /**
   * 检测解析类型
   */
  static detectParseType(content: ClipboardContent, availableFormats?: string[]): string {
    const formats = availableFormats || ClipboardDataConverter.getAvailableFormats(content)
    const primaryType = ClipboardDataConverter.detectPrimaryType(content, formats)

    // 图片类型优先级最高
    if (content.image) {
      return 'image'
    }

    // 当 availableFormats 中存在 rtf 时，RTF 优先级高于 html 和 text
    if (formats.includes('rtf') && content.rtf) {
      return 'rtf'
    }

    // 高优先级文本类型检测（在 HTML 和 bookmark 之前）
    if (content.text) {
      const text = content.text.trim()
      // 检测颜色值 (#xxxxxx 或 #xxxxxxxx)
      const colorRegex = /^#[0-9a-fA-F]{6}$|^#[0-9a-fA-F]{8}$/
      if (colorRegex.test(text)) {
        return 'color'
      }
      // 检测URL
      const urlRegex = /^https?:\/\/[^\s]+$/i
      if (urlRegex.test(text)) {
        return 'url'
      }
      // 检测文件路径（只检测 file:// 开头的内容，支持多个文件换行分隔）
      const lines = text.split('\n').map(line => line.trim()).filter(line => line.length > 0)
      const allLinesAreFiles = lines.length > 0 && lines.every(line => line.startsWith('file://'))
      if (allLinesAreFiles) {
        return 'file_path'
      }
      // 检测邮箱
      const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
      if (emailRegex.test(text)) {
        return 'email'
      }
    }

    // HTML 类型处理
    if (content.html) {
      return 'html'
    }

    // 书签类型
    if (content.bookmark) {
      return 'bookmark'
    }

    // 其他文本类型检测
    if (content.text) {
      const text = content.text.trim()
      // 检测电话号码
      const phoneRegex = /^1[3-9]\d{9}$/
      if (phoneRegex.test(text.replace(/\s/g, ''))) {
        return 'phone'
      }
      // 检测代码（简单检测）
      if (text.includes('{') && text.includes('}') ||
          text.includes('function') ||
          text.includes('class ') ||
          text.includes('import ') ||
          text.includes('export ')) {
        return 'code'
      }
      return 'text'
    }

    // 如果检测结果为空，返回 primaryType
    return primaryType
  }

  /**
   * 将标签组记录转换为标签组对象
   */
  static tagRecordToTag(record: TagGroupRecord): TagGroup {
    return {
      id: record.id,
      name: record.name,
      color: record.color,
      description: record.description,
      createdAt: record.created_at,
      updatedAt: record.updated_at
    }
  }

  /**
   * 将标签组对象转换为数据库记录格式
   */
  static tagToRecordData(tag: TagGroup): TagGroupRecord {
    return {
      id: tag.id,
      name: tag.name,
      color: tag.color,
      description: tag.description,
      created_at: tag.createdAt,
      updated_at: tag.updatedAt
    }
  }

  /**
   * 将应用信息记录转换为应用信息对象
   */
  static appRecordToAppInfo(record: AppInfoRecord): AppInfo {
    return {
      id: record.id,
      application: record.application,
      path: record.path,
      icon: record.icon,
      createdAt: record.created_at,
      updatedAt: record.updated_at
    }
  }

  /**
   * 将应用信息对象转换为数据库记录格式
   */
  static appInfoToRecordData(appInfo: AppInfo): AppInfoRecord {
    return {
      id: appInfo.id,
      application: appInfo.application,
      path: appInfo.path,
      icon: appInfo.icon,
      created_at: appInfo.createdAt,
      updated_at: appInfo.updatedAt
    }
  }

  /**
   * 生成应用信息ID
   */
  static generateAppId(): string {
    return `app_${Date.now()}_${Math.random().toString(36).substring(2, 11)}`
  }

  /**
   * 批量转换数据库记录到剪贴板项
   * 统一处理标签转换逻辑
   */
  static batchRecordsToItems(records: Array<ClipboardHistoryRecord & { tags?: TagGroupRecord[], appInfo?: AppInfoRecord }>): ClipboardItem[] {
    return records.map(record => {
      const tags = record.tags?.map(tag => this.tagRecordToTag(tag)) || []
      return this.recordToItem(record, tags)
    })
  }

  /**
   * 批量转换剪贴板项到数据库记录格式
   */
  static batchItemsToRecordData(items: ClipboardItem[]): Array<{
    content_id: string
    content_text?: string
    content_html?: string
    content_image?: string
    content_rtf?: string
    content_bookmark?: string
    primary_type: string
    available_formats: string
    timestamp: number
    last_used: number
    parse_type: string
    source_app?: string
  }> {
    return items.map(item => this.itemToRecordData(item))
  }
}
