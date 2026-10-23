import type { StoreWriteResult, StoreValueChange, StoreWriteMetadata } from '@shared/store-sync'
import type { OCRSettings, OCRModelStatus, OCRModelId } from '@shared/ocr-models'
import type { CopyStatistics, CopyStatisticsQuery } from '@shared/clipboard-statistics'
import type { MCPSettings, MCPState, MCPStatus } from '@shared/mcp'
import type {
  AIOCRAPIRequest,
  AIOCRModelOption,
  AIOCRUsageQuery,
  AIOCRUsageStats,
  OCRTaskStatus
} from '@shared/ai-ocr'
/**
 * IPC 通信类型定义
 * 为 IPC 频道和消息提供类型安全
 */

import type { IpcRendererEvent } from 'electron'
import type { ScheduledSyncStatus } from '@main/sync/types'
export type { IpcRendererListener } from '@preload/index.d'

// 基础类型
export type SimpleListener<Args extends unknown[] = unknown[]> = (...args: Args) => void

// 常用 IPC 频道定义
export type OverlayPosition = 'top' | 'bottom' | 'left' | 'right'

export interface IpcChannels {
  'mcp:get-state': void
  'mcp:save-settings': MCPSettings
  'mcp:generate-key': void
  'mcp:status': MCPStatus
  'statistics:get-copy': CopyStatisticsQuery | undefined
  'statistics:clear-copy': void
  'statistics:copy-updated': void
  'ocr:list-models': void
  'ocr:download-model': OCRModelId
  'ocr:cancel-download': OCRModelId
  'ocr:delete-model': OCRModelId
  'ocr:redownload-model': OCRModelId
  'ocr:ai-list-models': AIOCRAPIRequest
  'ocr:ai-test-connection': AIOCRAPIRequest
  'ocr:get-usage': AIOCRUsageQuery | undefined
  'ocr:clear-usage': void
  'ocr:usage-updated': void
  'ocr:task-status': OCRTaskStatus
  'ocr:model-status': OCRModelStatus

  // 窗口相关
  'open-overlay-window': void
  'close-overlay-window': void
  'open-overlay-finished': void

  // 动画相关
  'trigger-slide-in-animation': OverlayPosition
  'overlay-position-updated': OverlayPosition
  'trigger-slide-out-animation': void
  'trigger-close-window': void

  // 存储相关
  'store:patch': { storeId: string; patch: Record<string, unknown>; metadata?: StoreWriteMetadata }
  'store-value-changed': StoreValueChange

  // 剪贴板相关
  'clipboard-get-history': ClipboardHistoryQueryOptions | { limit?: number } | void
  'clipboard-restore': { itemId: string }
  'clipboard-restore-as-text': { itemId: string }
  'clipboard-restore-ocr-text': { itemId: string }
  'clipboard-parse-ocr': { itemId: string }
  'clipboard-parse-qrcode': { itemId: string }
  'clipboard-generate-qrcode': { itemId: string }
  'clipboard-delete-item': { itemId: string }
  'clipboard-clear-history': void
  'clipboard-get-status': void
  'clipboard-start-listening': void
  'clipboard-stop-listening': void
  'clipboard-update-config': { config: Partial<ClipboardManagerConfig> }

  // 标签相关
  'get-all-tags': void
  'create-tag': { name: string; color: string; description?: string }
  'update-tag': { tagId: string; updates: { name?: string; color?: string; description?: string } }
  'delete-tag': string
  'get-tag-stats': void
  'bind-tag-to-record': { contentId: string; tagId: string }
  'unbind-tag-from-record': { contentId: string; tagId: string }

  // 应用信息相关
  'get-all-apps': void
  'get-app-cache-stats': void

  // 应用控制相关
  'app:restart': void
  'app:window-minimize': void
  'app:window-close': void
}

// IPC 响应类型
export interface IpcResponses {
  'mcp:get-state': { success: boolean; data?: MCPState; error?: string }
  'mcp:save-settings': { success: boolean; data?: MCPState; error?: string }
  'mcp:generate-key': { success: boolean; data?: string; error?: string }
  'statistics:get-copy': { success: boolean; data?: CopyStatistics; error?: string }
  'statistics:clear-copy': { success: boolean; error?: string }
  'ocr:list-models': { success: boolean; data?: OCRModelStatus[]; error?: string }
  'ocr:download-model': { success: boolean; error?: string }
  'ocr:cancel-download': { success: boolean; error?: string }
  'ocr:delete-model': { success: boolean; error?: string }
  'ocr:redownload-model': { success: boolean; error?: string }
  'ocr:ai-list-models': { success: boolean; data?: AIOCRModelOption[]; error?: string }
  'ocr:ai-test-connection': {
    success: boolean
    data?: { latencyMs: number; model: string }
    error?: string
  }
  'ocr:get-usage': { success: boolean; data?: AIOCRUsageStats; error?: string }
  'ocr:clear-usage': { success: boolean; data?: AIOCRUsageStats; error?: string }

  'scheduled-sync:test-connection': boolean | { success: false; error: string }
  'scheduled-sync:manual-sync': { success: boolean; message?: string; error?: string }
  'scheduled-sync:manual-sync-with-config': { success: boolean; message?: string; error?: string }
  'scheduled-sync:get-status': { success: boolean; data?: ScheduledSyncStatus; error?: string }

  'store:patch': StoreWriteResult

  // 剪贴板相关响应
  'clipboard-get-history': { success: boolean; data?: ClipboardItem[]; error?: string }
  'clipboard-restore': { success: boolean; message?: string; error?: string }
  'clipboard-restore-as-text': { success: boolean; message?: string; error?: string }
  'clipboard-restore-ocr-text': { success: boolean; message?: string; error?: string }
  'clipboard-parse-ocr': { success: boolean; message?: string; error?: string }
  'clipboard-generate-qrcode': {
    success: boolean
    message?: string
    error?: string
    data?: { qrCodeDataUrl: string }
  }
  'clipboard-delete-item': { success: boolean; message?: string; error?: string }
  'clipboard-clear-history': { success: boolean; message?: string; error?: string }
  'clipboard-get-status': { success: boolean; data?: ClipboardStatus; error?: string }
  'clipboard-start-listening': { success: boolean; message?: string; error?: string }
  'clipboard-stop-listening': { success: boolean; message?: string; error?: string }
  'clipboard-update-config': { success: boolean; message?: string; error?: string }

  // 标签相关响应
  'get-all-tags': { success: boolean; data?: TagGroup[]; error?: string }
  'create-tag': { success: boolean; data?: { tagId: string }; message?: string; error?: string }
  'update-tag': { success: boolean; message?: string; error?: string }
  'delete-tag': { success: boolean; message?: string; error?: string }
  'get-tag-stats': { success: boolean; data?: TagStats[]; error?: string }

  // 应用信息相关响应
  'get-all-apps': { success: boolean; data?: AppInfo[]; error?: string }
  'get-app-cache-stats': { success: boolean; data?: AppCacheStats; error?: string }

  // 应用控制相关响应
  'app:restart': { success: boolean; error?: string }
  'app:window-minimize': { success: boolean; error?: string }
  'app:window-close': { success: boolean; error?: string }
}

// 类型安全的 IPC 方法接口
export interface TypedIpcRenderer {
  // 监听方法
  on<K extends keyof IpcChannels>(
    channel: K,
    listener: (event: IpcRendererEvent, data: IpcChannels[K]) => void
  ): () => void

  onSimple<K extends keyof IpcChannels>(
    channel: K,
    listener: (data: IpcChannels[K]) => void
  ): () => void

  once<K extends keyof IpcChannels>(
    channel: K,
    listener: (event: IpcRendererEvent, data: IpcChannels[K]) => void
  ): () => void

  onceSimple<K extends keyof IpcChannels>(
    channel: K,
    listener: (data: IpcChannels[K]) => void
  ): () => void

  // 发送方法
  send<K extends keyof IpcChannels>(channel: K, data: IpcChannels[K]): void

  // 调用方法
  invoke<K extends keyof IpcResponses>(
    channel: K,
    data: K extends keyof IpcChannels ? IpcChannels[K] : unknown
  ): Promise<IpcResponses[K]>

  // 同步调用
  sendSync<K extends keyof IpcResponses>(
    channel: K,
    data: K extends keyof IpcChannels ? IpcChannels[K] : unknown
  ): IpcResponses[K]

  // 移除监听器
  removeListener(
    channel: string,
    listener: (event: IpcRendererEvent, ...args: unknown[]) => void
  ): void
  removeAllListeners(channel: string): void
}

// 剪贴板内容数据类型（前端版本 - 图片数据为 Uint8Array）
export interface ClipboardContent {
  text?: string
  html?: string
  image?: Uint8Array // 图片原始数据（Uint8Array 格式）
  rtf?: string
  files?: string[] // 文件路径列表
  ocr?: string // OCR识别的文本内容
}

// 应用信息类型
export interface AppInfo {
  id: string
  application: string
  path: string
  icon?: string // base64 编码的应用图标
  createdAt: number
  updatedAt: number
}

// 应用缓存统计信息类型
export interface AppCacheStats {
  cacheSize: number
  maxCacheSize: number
}

// 标签组类型
export interface TagGroup {
  id: string
  name: string
  color: string
  description?: string
  createdAt: number
  updatedAt: number
}

// 标签统计信息类型
export interface TagStats {
  tagId: string
  tagName: string
  recordCount: number
}

// 剪贴板历史记录项类型
export interface ClipboardItem {
  id: string
  content: ClipboardContent
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

// 剪贴板历史查询选项
export interface ClipboardHistoryQueryOptions {
  limit?: number
  offset?: number
  tagIds?: string[]
  primaryType?: string
  searchText?: string
  orderBy?: 'timestamp' | 'last_used'
  orderDirection?: 'ASC' | 'DESC'
}

// 剪贴板管理器配置类型
export interface ClipboardManagerConfig {
  maxHistorySize: number
  enableAutoSave: boolean
  enableLocalOCR: boolean
  ocrMode: OCRSettings['ocrMode']
  ocrAIConfigs: OCRSettings['ocrAIConfigs']
  ocrLocalModel: OCRModelId
  ocrRemoteConfig: {
    url: string
    extraParams: string
    imageFieldName: string
    requestFormat: 'json' | 'form'
    resultPath: string
  }
}

// 剪贴板状态类型
export interface ClipboardStatus {
  isListening: boolean
  config: ClipboardManagerConfig
  historyCount: number
}

// 剪贴板 IPC 响应类型
export interface ClipboardIpcResponse<T = unknown> {
  success: boolean
  data?: T
  apps?: Record<string, AppInfo> // 应用信息映射
  message?: string
  error?: string
}

// 剪贴板历史查询结果类型
export interface ClipboardHistoryResult {
  items: ClipboardItem[]
  apps: Record<string, AppInfo>
}

// 剪贴板操作类型
export interface ClipboardOperations {
  getHistory(options?: ClipboardHistoryQueryOptions | number): Promise<ClipboardHistoryResult>
  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardHistoryResult>
  restore(itemId: string): Promise<boolean>
  restoreAsText(itemId: string): Promise<boolean>
  restoreOcrText(itemId: string): Promise<boolean>
  parseOcr(itemId: string): Promise<boolean>
  parseQRCode(itemId: string): Promise<boolean>
  generateQRCode(
    itemId: string
  ): Promise<{ success: boolean; message?: string; data?: { qrCodeDataUrl: string } }>
  deleteItem(itemId: string): Promise<boolean>
  clearHistory(): Promise<boolean>
  getStatus(): Promise<ClipboardStatus>
  startListening(): Promise<boolean>
  stopListening(): Promise<boolean>
  updateConfig(config: Partial<ClipboardManagerConfig>): Promise<boolean>
}

// 标签操作类型
export interface TagOperations {
  getAllTags(): Promise<TagGroup[]>
  createTag(name: string, color: string, description?: string): Promise<string>
  updateTag(
    tagId: string,
    updates: { name?: string; color?: string; description?: string }
  ): Promise<boolean>
  deleteTag(tagId: string): Promise<boolean>
  getTagStats(): Promise<TagStats[]>
  bindTagToRecord(contentId: string, tagId: string): Promise<boolean>
  unbindTagFromRecord(contentId: string, tagId: string): Promise<boolean>
}

// 应用信息操作类型
export interface AppOperations {
  getAllApps(): Promise<AppInfo[]>
  getCacheStats(): Promise<AppCacheStats>
}

// 应用控制操作类型
export interface AppControlOperations {
  restart(): Promise<boolean>
  minimizeWindow(): Promise<boolean>
  closeWindow(): Promise<boolean>
}

// 完整的 API 接口
export interface RendererAPI {
  // 原始 IPC 方法
  ipc: TypedIpcRenderer

  // 剪贴板操作
  clipboard: ClipboardOperations

  // 标签操作
  tags: TagOperations

  // 应用信息操作
  apps: AppOperations

  // 应用控制操作
  app: AppControlOperations
}
