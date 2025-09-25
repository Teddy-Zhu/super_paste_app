import { HashUtils } from '../utils/common'

/**
 * 剪贴板内容数据
 */
export interface ClipboardContent {
  text?: string
  html?: string
  image?: Buffer // 图片原始数据 (Buffer)
  rtf?: string
  files?: string[] // 文件路径列表
  ocr?: string // OCR识别的文本内容
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
 * 前端使用的剪贴板内容格式（图片为 Uint8Array）
 */
export interface ClipboardContentForUI {
  text?: string
  html?: string
  image?: Uint8Array // 图片原始数据（Uint8Array 格式，用于前端显示）
  rtf?: string
  files?: string[] // 文件路径列表
  ocr?: string // OCR识别的文本内容
}

/**
 * 剪贴板历史记录项（内存中使用）
 * 支持两种内容格式：内部处理用的 Buffer 格式和前端显示用的 base64 字符串格式
 */
export interface ClipboardItem {
  id: string
  content: ClipboardContent | ClipboardContentForUI
  primaryType:
    | 'text'
    | 'html'
    | 'image'
    | 'rtf'
    | 'files'
    | 'url'
    | 'email'
    | 'phone'
    | 'code'
    | 'color'
    | 'empty'
  availableFormats: string[]
  timestamp: number
  lastUsed: number
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
  content_image?: Buffer
  content_rtf?: string
  content_files?: string // JSON 字符串，存储文件路径数组
  content_ocr?: string // OCR识别的文本内容，用于搜索
  primary_type: string
  available_formats: string
  timestamp: number
  last_used: number
  source_app?: string // 来源应用ID
}

/**
 * 剪贴板管理器配置
 */
export interface ClipboardManagerConfig {
  maxHistorySize: number
  checkInterval: number
  enableAutoSave: boolean
  enableLocalOCR: boolean
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
  primaryType?: string // 按主要类型筛选
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
   * 注意：为了前端显示，图片 Buffer 数据会被转换为 Uint8Array
   */
  static recordToItem(record: ClipboardHistoryRecord, tags?: TagGroup[]): ClipboardItem {
    const content: any = {} // 使用 any 来避免类型冲突，因为前端需要 Uint8Array
    if (record.content_text) content.text = record.content_text
    if (record.content_html) content.html = record.content_html
    if (record.content_image) {
      // 将 Buffer 转换为 Uint8Array 供前端显示
      content.image = new Uint8Array(record.content_image)
    }
    if (record.content_rtf) content.rtf = record.content_rtf
    if (record.content_files) {
      try {
        content.files = JSON.parse(record.content_files)
      } catch {
        content.files = []
      }
    }
    if (record.content_ocr) content.ocr = record.content_ocr

    return {
      id: record.content_id,
      content,
      primaryType: record.primary_type as ClipboardItem['primaryType'],
      availableFormats: JSON.parse(record.available_formats),
      timestamp: record.timestamp,
      lastUsed: record.last_used,
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
    content_image?: Buffer
    content_rtf?: string
    content_files?: string
    content_ocr?: string
    primary_type: string
    available_formats: string
    timestamp: number
    last_used: number
    source_app?: string
  } {
    // 处理图片数据：支持 Buffer、Uint8Array 和 base64 字符串格式
    let imageBuffer: Buffer | undefined = undefined
    if (item.content.image) {
      if (Buffer.isBuffer(item.content.image)) {
        imageBuffer = item.content.image
      } else if (item.content.image instanceof Uint8Array) {
        // 从 Uint8Array 转换为 Buffer
        imageBuffer = Buffer.from(item.content.image)
      } else if (typeof item.content.image === 'string') {
        imageBuffer = Buffer.from(item.content.image, 'base64')
      }
    }

    return {
      content_id: item.id,
      content_text: item.content.text,
      content_html: item.content.html,
      content_image: imageBuffer,
      content_rtf: item.content.rtf,
      content_files: item.content.files ? JSON.stringify(item.content.files) : undefined,
      content_ocr: item.content.ocr,
      primary_type: item.primaryType,
      available_formats: JSON.stringify(item.availableFormats),
      timestamp: item.timestamp,
      last_used: item.lastUsed,
      source_app: item.source_app
    }
  }

  /**
   * 检测主要内容类型（合并了原来的 parseType 逻辑）
   */
  static detectPrimaryType(
    content: ClipboardContent,
    availableFormats?: string[]
  ): ClipboardItem['primaryType'] {
    const formats = availableFormats || ClipboardDataConverter.getAvailableFormats(content)

    // 文件类型优先级最高
    if (content.files && content.files.length > 0) return 'files'

    // 图片类型
    if (content.image) return 'image'

    // 当 availableFormats 中存在 rtf 时，RTF 优先级高于 html
    if (formats.includes('rtf') && content.rtf) return 'rtf'

    // HTML 类型处理
    if (content.html) return 'html'

    // 高优先级文本类型检测
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

      // 检测邮箱
      const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/
      if (emailRegex.test(text)) {
        return 'email'
      }

      // 检测电话号码
      const phoneRegex = /^1[3-9]\d{9}$/
      if (phoneRegex.test(text.replace(/\s/g, ''))) {
        return 'phone'
      }

      // 检测代码（简单检测）
      if (
        (text.includes('{') && text.includes('}')) ||
        text.includes('function') ||
        text.includes('class ') ||
        text.includes('import ') ||
        text.includes('export ')
      ) {
        return 'code'
      }

      return 'text'
    }

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
    if (content.files && content.files.length > 0) formats.push('files')
    return formats
  }

  /**
   * 检查内容是否为空
   */
  static isEmptyContent(content: ClipboardContent): boolean {
    return (
      !content.text &&
      !content.html &&
      !content.image &&
      !content.rtf &&
      (!content.files || content.files.length === 0)
    )
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
    if (content.files) optimizedContent.files = content.files
    if (content.ocr) optimizedContent.ocr = content.ocr

    // 对图片内容进行优化处理
    if (content.image) {
      const imageHash = ClipboardDataConverter.generateImageHash(content.image)
      optimizedContent.image = Buffer.from(imageHash, 'utf8')
    }

    return HashUtils.generateContentHash(optimizedContent)
  }

  /**
   * 生成图片内容的轻量级哈希
   * 使用采样策略而不是完整内容，大幅减少CPU消耗
   */
  private static generateImageHash(imageData: Buffer): string {
    if (!imageData || imageData.length === 0) return ''

    const dataLength = imageData.length

    // 小图片（<= 4KB）直接使用完整数据
    if (dataLength <= 4096) {
      return imageData.toString('base64')
    }

    // 大图片使用采样策略：
    // 1. 数据头部（包含格式信息）
    // 2. 数据长度
    // 3. 每隔一定间隔采样
    // 4. 数据尾部
    const head = imageData.subarray(0, 512).toString('base64') // 头部512字节
    const tail = imageData.subarray(dataLength - 512).toString('base64') // 尾部512字节

    // 中间部分采样：每隔1000字节取32字节
    let samples = ''
    const sampleInterval = 1000
    const sampleSize = 32
    for (let i = 1000; i < dataLength - 1000; i += sampleInterval) {
      const end = Math.min(i + sampleSize, dataLength)
      samples += imageData.subarray(i, end).toString('base64')
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
  static batchRecordsToItems(
    records: Array<ClipboardHistoryRecord & { tags?: TagGroupRecord[]; appInfo?: AppInfoRecord }>
  ): ClipboardItem[] {
    return records.map((record) => {
      const tags = record.tags?.map((tag) => this.tagRecordToTag(tag)) || []
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
    content_image?: Buffer
    content_rtf?: string
    content_files?: string
    content_ocr?: string
    primary_type: string
    available_formats: string
    timestamp: number
    last_used: number
    source_app?: string
  }> {
    return items.map((item) => this.itemToRecordData(item))
  }
}
