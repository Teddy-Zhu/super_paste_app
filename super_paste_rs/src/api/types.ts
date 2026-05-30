/**
 * API 类型定义 - Tauri 版本
 * 保持与原项目相同的接口契约
 */

export type OverlayPosition = 'top' | 'bottom' | 'left' | 'right'

export interface StoreOperations {
  get<T = any>(key: string, defaultValue?: T): Promise<T>
  set<T = any>(key: string, value: T): Promise<void>
}

export interface ClipboardContent {
  text?: string
  html?: string
  image?: Uint8Array
  rtf?: string
  files?: string[]
  ocr?: string
}

export interface AppInfo {
  id: string
  application: string
  path: string
  icon?: string
  createdAt: number
  updatedAt: number
}

export interface AppCacheStats {
  cacheSize: number
  maxCacheSize: number
}

export interface TagGroup {
  id: string
  name: string
  color: string
  description?: string
  createdAt: number
  updatedAt: number
}

export interface TagStats {
  tagId: string
  tagName: string
  recordCount: number
}

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
  tags?: TagGroup[]
  source_app?: string
}

export interface ClipboardHistoryQueryOptions {
  limit?: number
  offset?: number
  tagIds?: string[]
  primaryType?: string
  searchText?: string
  orderBy?: 'timestamp' | 'last_used'
  orderDirection?: 'ASC' | 'DESC'
}

export interface ClipboardManagerConfig {
  maxHistorySize: number
  enableAutoSave: boolean
  enableLocalOCR: boolean
  ocrMode: 'local' | 'remote'
  ocrRemoteConfig: {
    url: string
    extraParams: string
    imageFieldName: string
    requestFormat: 'json' | 'form'
    resultPath: string
  }
}

export interface ClipboardStatus {
  isListening: boolean
  config: ClipboardManagerConfig
  historyCount: number
}

export interface ClipboardIpcResponse<T = any> {
  success: boolean
  data?: T
  apps?: Record<string, AppInfo>
  message?: string
  error?: string
}

export interface ClipboardHistoryResult {
  items: ClipboardItem[]
  apps: Record<string, AppInfo>
}

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

export interface AppOperations {
  getAllApps(): Promise<AppInfo[]>
  getCacheStats(): Promise<AppCacheStats>
}

export interface AppControlOperations {
  restart(): Promise<boolean>
  minimizeWindow(): Promise<boolean>
  closeWindow(): Promise<boolean>
}
