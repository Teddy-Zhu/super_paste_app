/**
 * IPC 通信类型定义
 * 为 IPC 频道和消息提供类型安全
 */

import type { IpcRendererEvent } from 'electron'

// 基础类型
export type IpcRendererListener = (event: IpcRendererEvent, ...args: any[]) => void
export type SimpleListener = (...args: any[]) => void

// 常用 IPC 频道定义
export interface IpcChannels {
  // 窗口相关
  'open-overlay-window': void
  'close-overlay-window': void
  'open-overlay-finished': void


  // 动画相关
  'trigger-slide-in-animation': void
  'trigger-slide-out-animation': void
  'trigger-close-window': void

  // 存储相关
  'store-get': { key: string; defaultValue?: any }
  'store-set': { key: string; value: any }
  'store-value-changed': { key: string; value: any; timestamp: number }

  // 剪贴板相关
  'clipboard-get-history': ClipboardHistoryQueryOptions | { limit?: number } | void
  'clipboard-restore': { itemId: string }
  'clipboard-restore-as-text': { itemId: string }
  'clipboard-delete-item': { itemId: string }
  'clipboard-clear-history': void
  'clipboard-get-status': void
  'clipboard-start-listening': void
  'clipboard-stop-listening': void
  'clipboard-update-config': { config: any }

  // 标签相关
  'get-all-tags': void
  'create-tag': { name: string; color: string; description?: string }
  'update-tag': { tagId: string; updates: { name?: string; color?: string; description?: string } }
  'delete-tag': string

  // 应用信息相关
  'get-all-apps': void
  'get-app-cache-stats': void

}

// IPC 响应类型
export interface IpcResponses {
  'store-get': { success: boolean; data?: any; error?: string }
  'store-set': { success: boolean; error?: string }

  // 剪贴板相关响应
  'clipboard-get-history': { success: boolean; data?: any[]; error?: string }
  'clipboard-restore': { success: boolean; message?: string; error?: string }
  'clipboard-delete-item': { success: boolean; message?: string; error?: string }
  'clipboard-clear-history': { success: boolean; message?: string; error?: string }
  'clipboard-get-status': { success: boolean; data?: any; error?: string }
  'clipboard-start-listening': { success: boolean; message?: string; error?: string }
  'clipboard-stop-listening': { success: boolean; message?: string; error?: string }
  'clipboard-update-config': { success: boolean; message?: string; error?: string }

  // 标签相关响应
  'get-all-tags': { success: boolean; data?: TagGroup[]; error?: string }
  'create-tag': { success: boolean; data?: { tagId: string }; message?: string; error?: string }
  'update-tag': { success: boolean; message?: string; error?: string }
  'delete-tag': { success: boolean; message?: string; error?: string }

  // 应用信息相关响应
  'get-all-apps': { success: boolean; data?: AppInfo[]; error?: string }
  'get-app-cache-stats': { success: boolean; data?: AppCacheStats; error?: string }

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
    data: K extends keyof IpcChannels ? IpcChannels[K] : any
  ): Promise<IpcResponses[K]>
  
  // 同步调用
  sendSync<K extends keyof IpcResponses>(
    channel: K,
    data: K extends keyof IpcChannels ? IpcChannels[K] : any
  ): IpcResponses[K]
  
  // 移除监听器
  removeListener(channel: string, listener: (...args: any[]) => void): void
  removeAllListeners(channel: string): void
}

// 存储操作类型
export interface StoreOperations {
  get<T = any>(key: string, defaultValue?: T): Promise<T>
  set<T = any>(key: string, value: T): Promise<void>
}

// 剪贴板内容数据类型
export interface ClipboardContent {
  text?: string
  html?: string
  image?: string // base64 编码的图片数据
  rtf?: string
  bookmark?: string // JSON 字符串
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

// 剪贴板历史记录项类型
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

// 剪贴板历史查询选项
export interface ClipboardHistoryQueryOptions {
  limit?: number
  offset?: number
  tagIds?: string[]
  parseType?: string
  searchText?: string
  orderBy?: 'timestamp' | 'last_used'
  orderDirection?: 'ASC' | 'DESC'
}

// 剪贴板管理器配置类型
export interface ClipboardManagerConfig {
  maxHistorySize: number
  checkInterval: number
  enableAutoSave: boolean
}

// 剪贴板状态类型
export interface ClipboardStatus {
  isListening: boolean
  config: ClipboardManagerConfig
  historyCount: number
}

// 剪贴板 IPC 响应类型
export interface ClipboardIpcResponse<T = any> {
  success: boolean
  data?: T
  message?: string
  error?: string
}



// 剪贴板操作类型
export interface ClipboardOperations {
  getHistory(options?: ClipboardHistoryQueryOptions | number): Promise<ClipboardItem[]>
  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardItem[]>
  restore(itemId: string): Promise<boolean>
  restoreAsText(itemId: string): Promise<boolean>
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
  updateTag(tagId: string, updates: { name?: string; color?: string; description?: string }): Promise<boolean>
  deleteTag(tagId: string): Promise<boolean>
}

// 应用信息操作类型
export interface AppOperations {
  getAllApps(): Promise<AppInfo[]>
  getCacheStats(): Promise<AppCacheStats>
}


// 完整的 API 接口
export interface RendererAPI extends
  StoreOperations {

  // 原始 IPC 方法
  ipc: TypedIpcRenderer

  // 剪贴板操作
  clipboard: ClipboardOperations

  // 标签操作
  tags: TagOperations

  // 应用信息操作
  apps: AppOperations

}
