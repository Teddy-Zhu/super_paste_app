import type { Transferable } from 'node:worker_threads'
import type {
  ClipboardContent,
  ClipboardManagerConfig,
  CleanupConfig,
  CleanupStats,
  ClipboardHistoryRecord,
  TagGroup,
  TagStats,
  QueryOptions
} from '../features/clipboard/types/clipboard-types'
import type { SyncResult } from '../sync/types'

/**
 * Worker Thread 通信类型定义
 */

// 基础消息类型
export interface BaseWorkerMessage {
  id: string
  type: string
  timestamp: number
}

// 请求消息
export interface WorkerRequest extends BaseWorkerMessage {
  type: 'request'
  method: string
  params?: unknown
}

// 响应消息
export interface WorkerResponse extends BaseWorkerMessage {
  type: 'response'
  requestId: string
  success: boolean
  data?: unknown
  error?: string
}

// 事件消息
export interface WorkerEvent extends BaseWorkerMessage {
  type: 'event'
  event: string
  data?: unknown
}

// 所有消息类型的联合
export type WorkerMessage = WorkerRequest | WorkerResponse | WorkerEvent

// 历史记录清理器 Worker 方法
export interface ClipboardHistoryCleanerWorkerMethods {
  // 配置管理
  updateConfig: (config: {
    maxKeepTime: number
    maxHistorySize?: number
    enableCountBasedCleanup?: boolean
  }) => void
  getConfig: () => CleanupConfig

  // 清理操作
  startAutoCleanup: () => void
  stopAutoCleanup: () => void
  performCleanup: () => Promise<CleanupStats>
  checkCountBasedCleanup: (maxHistorySize: number) => Promise<void>

  // 生命周期
  initialize: () => void
  destroy: () => void
}

// 剪贴板管理器 Worker 方法
export interface ClipboardManagerWorkerMethods {
  // 初始化和配置
  initialize: (config?: Partial<ClipboardManagerConfig>) => Promise<void>
  updateConfig: (config: Partial<ClipboardManagerConfig>) => Promise<void>
  getConfig: () => ClipboardManagerConfig

  // 监听控制
  startListening: () => void
  stopListening: () => void
  isListeningActive: () => boolean

  // 剪贴板操作
  getCurrentClipboard: () => Promise<ClipboardContent>
  syncClipboardHash: () => void
  resetErrorState: () => void

  // 历史记录管理
  deleteHistoryItem: (itemId: string) => boolean
  clearHistory: () => void
  searchHistory: (options: QueryOptions) => ClipboardHistoryRecord[]

  // 标签管理
  createTagGroup: (name: string, color: string, description?: string) => string | null
  getAllTagGroups: () => TagGroup[]
  updateTagGroup: (
    tagId: string,
    updates: Partial<Pick<TagGroup, 'name' | 'color' | 'description'>>
  ) => boolean
  deleteTagGroup: (tagId: string) => boolean
  bindTagToRecord: (contentId: string, tagId: string) => boolean
  unbindTagFromRecord: (contentId: string, tagId: string) => boolean
  getRecordsByTags: (tagIds: string[]) => ClipboardHistoryRecord[]
  getRecordsByParseType: (parseType: string) => ClipboardHistoryRecord[]
  getTagStats: () => TagStats[]
}

// Worker 事件类型
export interface WorkerEvents {
  // 历史记录清理器事件
  'cleanup-completed': { stats: CleanupStats }
  'cleanup-error': { error: string }

  // 剪贴板管理器事件
  'clipboard-changed': { timestamp: number }
  'history-updated': { timestamp: number; source?: string }

  // 定时同步事件
  'sync-scheduled-started': { interval: number; nextSyncTime: number }
  'sync-scheduled-stopped': Record<string, never>
  'sync-scheduled-start-failed': { reason: string; timestamp: number }
  'sync-started': { timestamp: number }
  'sync-completed': { timestamp: number; result: SyncResult; nextSyncTime: number }
  'sync-failed': { timestamp: number; result: SyncResult; nextSyncTime: number }
  'sync-error': { timestamp: number; error: string }
  'manual-sync-started': { timestamp: number }
  'manual-sync-completed': { timestamp: number; result: SyncResult }
  'manual-sync-failed': { timestamp: number; result: SyncResult }
  'manual-sync-error': { timestamp: number; error: string }
  'connection-test-success': { timestamp: number }
  'connection-test-failed': { timestamp: number }
  'connection-test-error': { timestamp: number; error: string }

  // 通用事件
  'worker-ready': { workerName?: string }
  'worker-error': { error: string }
  'worker-destroyed': Record<string, never>
}

// Worker 配置
export interface WorkerConfig {
  name: string
  workerPath: string
  options?: {
    workerData?: unknown
    transferList?: Transferable[]
  }
}

// Worker 管理器接口
export interface WorkerManager {
  start(): Promise<void>
  stop(): Promise<void>
  call<T = unknown>(method: string, params?: unknown): Promise<T>
  on(event: string, listener: (data: unknown) => void): void
  off(event: string, listener: (data: unknown) => void): void
  isRunning(): boolean
}
