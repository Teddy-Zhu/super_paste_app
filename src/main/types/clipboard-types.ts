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
  sourceAppInfo?: AppInfo // 来源应用信息
}

/**
 * 数据库中的剪贴板历史记录项
 */
export interface ClipboardHistoryRecord {
  id: number
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
  cleanupInterval: number // 清理检查间隔（毫秒）
  enableAutoCleanup: boolean // 是否启用自动清理
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
  static recordToItem(record: ClipboardHistoryRecord, tags?: TagGroup[], sourceAppInfo?: AppInfo): ClipboardItem {
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
      sourceAppInfo: sourceAppInfo
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
      source_app: item.sourceAppInfo?.id
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
   */
  static generateContentHash(content: ClipboardContent): string {
    const contentStr = JSON.stringify(content)
    let hash = 0
    for (let i = 0; i < contentStr.length; i++) {
      const char = contentStr.charCodeAt(i)
      hash = ((hash << 5) - hash) + char
      hash = hash & hash
    }
    return hash.toString()
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
      console.log('text:', text)
      // 检测颜色值 (#xxxxxx 或 #xxxxxxxx)
      const colorRegex = /^#[0-9a-fA-F]{6}$|^#[0-9a-fA-F]{8}$/
      if (colorRegex.test(text)) {
        console.log('color:', text)
        return 'color'
      }
      // 检测URL
      const urlRegex = /^https?:\/\/[^\s]+$/i
      if (urlRegex.test(text)) {
        return 'url'
      }
      // 检测文件路径
      const filePathRegex = /^[a-zA-Z]:\\|^\/|^\.\/|^\.\.\/|.*\.[a-zA-Z0-9]+$/
      if (filePathRegex.test(text)) {
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
      // 检测是否为链接
      const htmlContent = content.html.toLowerCase()
      if (htmlContent.includes('<a ') && htmlContent.includes('href=')) {
        return 'link'
      }
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
      const phoneRegex = /^[\+]?[1-9][\d]{0,15}$|^[\+]?[(]?[\d\s\-\(\)]{10,}$/
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
}
