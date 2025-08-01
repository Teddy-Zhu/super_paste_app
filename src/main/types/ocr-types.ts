/**
 * OCR 相关类型定义
 */

/**
 * 阅读方向类型
 */
export type ReadingDirection = 'lr' | 'rl' | 'tb' | 'bt'

/**
 * 颜色类型 (RGB)
 */
export type RGBColor = [number, number, number]

/**
 * 坐标点类型
 */
export type Point = [number, number]

/**
 * 文字框类型 (四个角的坐标: ↖ ↗ ↘ ↙)
 */
export type TextBox = [Point, Point, Point, Point]

/**
 * OCR 识别结果中的单个文字区域
 */
export interface OCRTextRegion {
  /** 识别的文字内容 */
  text: string
  /** 置信度 (0-1) */
  mean: number
  /** 文字框坐标 */
  box: TextBox
  /** 样式信息 */
  style: {
    /** 背景色 */
    bg: RGBColor
    /** 文字色 */
    text: RGBColor
  }
}

/**
 * 段落信息
 */
export interface OCRParagraph {
  /** 段落中的文字区域 */
  src: OCRTextRegion[]
  /** 解析后的段落信息 */
  parse: OCRTextRegion
}

/**
 * 分栏信息
 */
export interface OCRColumn {
  /** 分栏中的文字区域 */
  src: OCRTextRegion[]
  /** 分栏的外边界框 */
  outerBox: TextBox
  /** 分栏中的段落 */
  parragraphs: OCRParagraph[]
}

/**
 * 阅读方向信息
 */
export interface ReadingDirectionInfo {
  /** 行内阅读方向 */
  inline: ReadingDirection
  /** 行间排版方向 */
  block: ReadingDirection
}

/**
 * 角度信息
 */
export interface AngleInfo {
  /** 阅读方向的具体角度 */
  reading: {
    /** 行内角度 */
    inline: number
    /** 行间角度 */
    block: number
  }
  /** 整体旋转角度 */
  angle: number
}

/**
 * 完整的 OCR 识别结果
 */
export interface OCRResult {
  /** 原始识别结果 - 每个视觉行 */
  src: OCRTextRegion[]
  /** 分栏信息 */
  columns: OCRColumn[]
  /** 聚合的段落信息 */
  parragraphs: OCRTextRegion[]
  /** 阅读方向信息 */
  readingDir: ReadingDirectionInfo
  /** 角度信息 */
  angle: AngleInfo
}

/**
 * OCR 配置选项
 */
export interface OCRConfiguration {
  /** 检测模型缩放比例，默认 1.0，小于1可提高速度但降低精度 */
  detRatio?: number
  /** 识别图片高度，默认 48 */
  recImgHeight?: number
  /** 是否启用空格优化，默认 true */
  enableSpaceOptimize?: boolean
  /** 是否启用文档方向识别，默认 false */
  enableDocCls?: boolean
  /** 是否启用调试模式 */
  debug?: boolean
}

/**
 * OCR 服务响应基础接口
 */
export interface OCRServiceResponse<T = any> {
  /** 操作是否成功 */
  success: boolean
  /** 错误信息（如果失败） */
  error?: string
  /** 响应数据 */
  data?: T
}

/**
 * 文字识别响应
 */
export interface OCRRecognitionResponse extends OCRServiceResponse<OCRResult> {
  /** 识别的文字内容 */
  text: string
  /** 平均置信度 */
  confidence: number
  /** 详细识别结果 */
  details?: OCRResult
}

/**
 * 文字检测响应
 */
export interface OCRDetectionResponse extends OCRServiceResponse {
  /** 检测到的文字区域 */
  regions: TextBox[]
}

/**
 * 批量识别结果项
 */
export interface BatchRecognitionItem {
  /** 图片索引 */
  index: number
  /** 识别的文字 */
  text: string
  /** 置信度 */
  confidence: number
  /** 是否成功 */
  success: boolean
  /** 错误信息 */
  error?: string
}

/**
 * 批量识别响应
 */
export interface BatchRecognitionResponse extends OCRServiceResponse {
  /** 批量识别结果 */
  results: BatchRecognitionItem[]
  /** 总处理数量 */
  totalProcessed: number
  /** 成功处理数量 */
  totalSuccess: number
}

/**
 * OCR 服务状态
 */
export interface OCRServiceStatus {
  /** 服务是否就绪 */
  isReady: boolean
  /** 资源文件路径 */
  resourcePath: string
  /** OCR 引擎是否已初始化 */
  ocrInitialized: boolean
  /** 支持的图片格式 */
  supportedFormats?: string[]
}

/**
 * 图片输入类型
 */
export type ImageInput = string | Buffer

/**
 * 支持的图片格式
 */
export type SupportedImageFormat = 'png' | 'jpg' | 'jpeg' | 'gif' | 'bmp' | 'webp'

/**
 * OCR 性能统计
 */
export interface OCRPerformanceStats {
  /** 总处理时间（毫秒） */
  totalTime: number
  /** 平均处理时间（毫秒） */
  averageTime: number
  /** 处理的图片数量 */
  processedCount: number
  /** 成功率 */
  successRate: number
  /** 每秒处理图片数 */
  imagesPerSecond: number
}

/**
 * OCR 错误类型
 */
export enum OCRErrorType {
  /** 服务未初始化 */
  SERVICE_NOT_INITIALIZED = 'SERVICE_NOT_INITIALIZED',
  /** 无效的图片格式 */
  INVALID_IMAGE_FORMAT = 'INVALID_IMAGE_FORMAT',
  /** 图片读取失败 */
  IMAGE_READ_FAILED = 'IMAGE_READ_FAILED',
  /** 模型文件不存在 */
  MODEL_FILE_NOT_FOUND = 'MODEL_FILE_NOT_FOUND',
  /** OCR 引擎错误 */
  OCR_ENGINE_ERROR = 'OCR_ENGINE_ERROR',
  /** 网络错误 */
  NETWORK_ERROR = 'NETWORK_ERROR',
  /** 内存不足 */
  OUT_OF_MEMORY = 'OUT_OF_MEMORY',
  /** 未知错误 */
  UNKNOWN_ERROR = 'UNKNOWN_ERROR'
}

/**
 * OCR 错误信息
 */
export interface OCRError {
  /** 错误类型 */
  type: OCRErrorType
  /** 错误消息 */
  message: string
  /** 错误详情 */
  details?: any
  /** 错误发生时间 */
  timestamp: Date
}

/**
 * OCR 事件类型
 */
export enum OCREventType {
  /** 初始化开始 */
  INITIALIZATION_STARTED = 'initialization_started',
  /** 初始化完成 */
  INITIALIZATION_COMPLETED = 'initialization_completed',
  /** 识别开始 */
  RECOGNITION_STARTED = 'recognition_started',
  /** 识别完成 */
  RECOGNITION_COMPLETED = 'recognition_completed',
  /** 检测开始 */
  DETECTION_STARTED = 'detection_started',
  /** 检测完成 */
  DETECTION_COMPLETED = 'detection_completed',
  /** 错误发生 */
  ERROR_OCCURRED = 'error_occurred'
}

/**
 * OCR 事件数据
 */
export interface OCREventData {
  /** 事件类型 */
  type: OCREventType
  /** 事件时间戳 */
  timestamp: Date
  /** 事件数据 */
  data?: any
  /** 错误信息（如果是错误事件） */
  error?: OCRError
}

/**
 * OCR 进度回调函数类型
 */
export type OCRProgressCallback = (progress: {
  /** 当前进度 (0-100) */
  percentage: number
  /** 当前步骤描述 */
  step: string
  /** 已完成的项目数 */
  completed: number
  /** 总项目数 */
  total: number
}) => void

/**
 * OCR 结果格式化选项
 */
export interface OCRFormatOptions {
  /** 输出格式 */
  format: 'text' | 'json' | 'markdown' | 'csv' | 'xml'
  /** 是否包含置信度信息 */
  includeConfidence?: boolean
  /** 是否包含坐标信息 */
  includeCoordinates?: boolean
  /** 是否包含样式信息 */
  includeStyle?: boolean
  /** 文本分隔符 */
  separator?: string
}

/**
 * OCR 缓存配置
 */
export interface OCRCacheConfig {
  /** 是否启用缓存 */
  enabled: boolean
  /** 缓存大小限制（MB） */
  maxSize: number
  /** 缓存过期时间（秒） */
  ttl: number
  /** 缓存键生成策略 */
  keyStrategy: 'hash' | 'filename' | 'custom'
}
