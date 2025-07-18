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
  params?: any
}

// 响应消息
export interface WorkerResponse extends BaseWorkerMessage {
  type: 'response'
  requestId: string
  success: boolean
  data?: any
  error?: string
}

// 事件消息
export interface WorkerEvent extends BaseWorkerMessage {
  type: 'event'
  event: string
  data?: any
}

// 所有消息类型的联合
export type WorkerMessage = WorkerRequest | WorkerResponse | WorkerEvent

// 历史记录清理器 Worker 方法
export interface ClipboardHistoryCleanerWorkerMethods {
  // 配置管理
  'updateConfig': (config: { maxKeepTime: number; maxHistorySize?: number; enableCountBasedCleanup?: boolean }) => void
  'getConfig': () => any

  // 清理操作
  'startAutoCleanup': () => void
  'stopAutoCleanup': () => void
  'performCleanup': () => Promise<any>
  'checkCountBasedCleanup': (maxHistorySize: number) => Promise<void>

  // 生命周期
  'initialize': () => void
  'destroy': () => void
}

// 剪贴板管理器 Worker 方法
export interface ClipboardManagerWorkerMethods {
  // 初始化和配置
  'initialize': (config?: any) => void
  'updateConfig': (config: any) => void
  'getConfig': () => any
  
  // 监听控制
  'startListening': () => void
  'stopListening': () => void
  'isListeningActive': () => boolean
  
  // 剪贴板操作
  'getCurrentClipboard': () => any
  'setClipboardContent': (content: any, updateHash?: boolean) => boolean
  'restoreClipboard': (itemId: string) => boolean
  'restoreClipboardAsText': (itemId: string) => boolean
  'syncClipboardHash': () => void
  'resetErrorState': () => void

  // 历史记录管理
  'getHistory': (limit?: number) => any[]
  'deleteHistoryItem': (itemId: string) => boolean
  'clearHistory': () => void
  'searchHistory': (options: any) => any[]
  
  // 标签管理
  'createTagGroup': (name: string, color: string, description?: string) => string | null
  'getAllTagGroups': () => any[]
  'updateTagGroup': (tagId: string, updates: any) => boolean
  'deleteTagGroup': (tagId: string) => boolean
  'bindTagToRecord': (contentId: string, tagId: string) => boolean
  'unbindTagFromRecord': (contentId: string, tagId: string) => boolean
  'getRecordsByTags': (tagIds: string[]) => any[]
  'getRecordsByParseType': (parseType: string) => any[]
  'getTagStats': () => any[]
  
  // 配置更新
  'updateMaxHistorySizeFromStore': () => void
  'updateMaxKeepTimeFromStore': () => void
  
  // 测试和验证
  'testClipboard': () => void
  'verifyClipboardContent': () => any
}

// Worker 事件类型
export interface WorkerEvents {
  // 历史记录清理器事件
  'cleanup-completed': { stats: any }
  'cleanup-error': { error: string }

  // 剪贴板管理器事件
  'clipboard-changed': { timestamp: number }
  'history-updated': { timestamp: number, source?: string }

  // 通用事件
  'worker-ready': {}
  'worker-error': { error: string }
  'worker-destroyed': {}
}

// Worker 配置
export interface WorkerConfig {
  name: string
  workerPath: string
  options?: {
    workerData?: any
    transferList?: any[]
  }
}

// Worker 管理器接口
export interface WorkerManager {
  start(): Promise<void>
  stop(): Promise<void>
  call<T = any>(method: string, params?: any): Promise<T>
  on(event: string, listener: (data: any) => void): void
  off(event: string, listener: (data: any) => void): void
  isRunning(): boolean
}
