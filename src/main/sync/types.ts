/**
 * 同步功能类型定义
 * 提供完整的 TypeScript 类型支持
 */

/**
 * 同步状态枚举
 */
export enum SyncStatus {
  IDLE = 'idle',
  SYNCING = 'syncing',
  SUCCESS = 'success',
  ERROR = 'error'
}

/**
 * 同步方向枚举
 */
export enum SyncDirection {
  UPLOAD = 'upload',
  DOWNLOAD = 'download',
  BIDIRECTIONAL = 'bidirectional'
}

/**
 * 同步提供者类型枚举
 */
export enum SyncProviderType {
  WEBDAV = 'webdav',
  FTP = 'ftp',
  SFTP = 'sftp',
  CLOUD_STORAGE = 'cloud_storage'
}

/**
 * 基础同步配置接口
 */
export interface SyncConfig {
  /** 是否启用同步 */
  enabled: boolean
  /** 是否启用自动同步 */
  autoSync: boolean
  /** 同步间隔（分钟） */
  syncInterval: number
  /** 同步方向 */
  syncDirection: SyncDirection
}

/**
 * WebDAV 同步配置接口
 */
export interface WebDAVConfig extends SyncConfig {
  /** WebDAV 服务器地址 */
  serverUrl: string
  /** 用户名 */
  username: string
  /** 密码 */
  password: string
  /** 远程路径 */
  remotePath: string
  /** 请求超时时间（毫秒） */
  timeout?: number
  /** 是否使用 HTTPS */
  useHttps?: boolean
}

/**
 * 同步文件信息接口
 */
export interface SyncFileInfo {
  /** 文件路径 */
  path: string
  /** 最后修改时间 */
  lastModified: Date
  /** 文件大小 */
  size: number
  /** 文件校验和（可选） */
  checksum?: string
}

/**
 * 同步结果接口
 */
export interface SyncResult {
  /** 是否成功 */
  success: boolean
  /** 结果消息 */
  message: string
  /** 上传的文件列表 */
  uploadedFiles?: string[]
  /** 下载的文件列表 */
  downloadedFiles?: string[]
  /** 错误信息列表 */
  errors?: string[]
}

/**
 * 同步事件接口
 */
export interface SyncEvent {
  /** 事件类型 */
  type: 'status-changed' | 'progress' | 'file-synced' | 'error' | 'config-updated'
  /** 事件数据 */
  data: any
}

/**
 * 同步事件监听器类型
 */
export type SyncEventListener = (event: SyncEvent) => void

/**
 * 同步提供者配置接口
 */
export interface SyncProviderConfig {
  /** 提供者类型 */
  type: SyncProviderType
  /** 提供者配置 */
  config: SyncConfig
}

/**
 * 完整的同步配置接口（包含启用状态）
 */
export interface CompleteSyncConfig extends SyncProviderConfig {
  /** 是否启用 */
  enabled: boolean
}

/**
 * 同步状态信息接口
 */
export interface SyncStatusInfo {
  /** 当前状态 */
  status: SyncStatus
  /** 是否启用自动同步 */
  isAutoSyncEnabled: boolean
  /** 最后同步时间 */
  lastSyncTime?: Date
  /** 下次同步时间 */
  nextSyncTime?: Date
}

/**
 * WebDAV 设置选项接口
 */
export interface WebDAVSetupOptions {
  /** 服务器地址 */
  serverUrl: string
  /** 用户名 */
  username: string
  /** 密码 */
  password: string
  /** 远程路径（可选） */
  remotePath?: string
  /** 是否启用自动同步（可选） */
  autoSync?: boolean
  /** 同步间隔（分钟，可选） */
  syncInterval?: number
  /** 同步方向（可选） */
  syncDirection?: SyncDirection
}

/**
 * IPC 消息类型定义
 */
export interface SyncIPCMessages {
  /** 设置 WebDAV 同步 */
  'sync:setup-webdav': {
    request: WebDAVSetupOptions
    response: boolean
  }
  /** 执行手动同步 */
  'sync:manual-sync': {
    request: void
    response: boolean
  }
  /** 测试连接 */
  'sync:test-connection': {
    request: void
    response: boolean
  }
  /** 获取同步状态 */
  'sync:get-status': {
    request: void
    response: SyncStatusInfo
  }
  /** 控制自动同步 */
  'sync:toggle-auto-sync': {
    request: boolean
    response: boolean
  }
  /** 禁用同步 */
  'sync:disable': {
    request: void
    response: boolean
  }
}

/**
 * 文件同步信息接口
 */
export interface FileSyncInfo {
  /** 操作类型 */
  action: 'upload' | 'download'
  /** 本地文件路径 */
  localPath?: string
  /** 远程文件路径 */
  remotePath?: string
  /** 时间戳 */
  timestamp: Date
}

/**
 * 同步进度信息接口
 */
export interface SyncProgressInfo {
  /** 当前文件 */
  currentFile: string
  /** 已完成文件数 */
  completed: number
  /** 总文件数 */
  total: number
  /** 进度百分比 */
  percentage: number
}

/**
 * 错误信息接口
 */
export interface SyncErrorInfo {
  /** 错误消息 */
  message: string
  /** 错误代码（可选） */
  code?: string
  /** 发生错误的文件（可选） */
  file?: string
  /** 时间戳 */
  timestamp: Date
}
