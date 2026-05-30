// Tauri 命令类型定义（与后端 Rust 类型对应）

export interface ClipboardItem {
  id: number;
  content_id: string;
  primary_type: string;
  content_hash: string;
  preview_text: string | null;
  source_app_id: number | null;
  is_pinned: boolean;
  is_favorite: boolean;
  created_at: string;
  last_used: string;
  // 兼容原项目字段
  content?: ClipboardItemContent;
  source_app?: string;
  tags?: string[];
}

export interface ClipboardItemContent {
  text?: string;
  html?: string;
  rtf?: string;
  image?: string; // base64
  files?: string[];
}

export interface ClipboardContent {
  content_id: string;
  text_content: string | null;
  html_content: string | null;
  rtf_content: string | null;
  image_data: number[] | null; // Vec<u8> 序列化为数组
  file_paths: string[] | null;
}

export interface QueryOptions {
  limit?: number;
  offset?: number;
  type_filter?: string;
  pinned_only?: boolean;
  favorite_only?: boolean;
}

export interface AppConfig {
  max_history_items: number;
  auto_paste_delay_ms: number;
  enable_ocr: boolean;
  enable_sync: boolean;
  sync_server_url: string;
  sync_username: string;
  sync_password: string;
  global_shortcut: string;
  overlay_position: string;
  theme: string;
  language: string;
}

// ==================== 兼容原项目的类型 ====================

export interface AppInfo {
  id: string;
  name: string;
  icon?: string;
  bundleId?: string;
}

export interface TagGroup {
  id: string;
  name: string;
  color: string;
  description?: string;
  count: number;
  created_at?: string;
  updated_at?: string;
}

export interface TagStats {
  tagId: string;
  tagName: string;
  count: number;
}

export interface ClipboardHistoryQueryOptions {
  limit?: number;
  offset?: number;
  tagIds?: string[];
  searchText?: string;
  orderBy?: string;
  orderDirection?: 'ASC' | 'DESC';
}

export interface ClipboardHistoryResult {
  items: ClipboardItem[];
  apps: Record<string, AppInfo>;
}

export interface ClipboardIpcResponse<T = any> {
  success: boolean;
  data?: T;
  apps?: Record<string, AppInfo>;
  error?: string;
  message?: string;
}

export interface ClipboardStatus {
  isListening: boolean;
  itemCount: number;
  lastUpdate?: string;
}

export interface ClipboardManagerConfig {
  maxHistorySize: number;
  enableOCR: boolean;
  enableSync: boolean;
}

// ==================== Store 操作类型 ====================

export interface StoreOperations {
  get<T = any>(key: string, defaultValue?: T): Promise<T>;
  set<T = any>(key: string, value: T): Promise<void>;
}

// ==================== Clipboard 操作类型 ====================

export interface ClipboardOperations {
  getHistory(
    options?: ClipboardHistoryQueryOptions | number
  ): Promise<ClipboardHistoryResult>;
  searchHistory(options: ClipboardHistoryQueryOptions): Promise<ClipboardHistoryResult>;
  restore(itemId: string): Promise<boolean>;
  restoreAsText(itemId: string): Promise<boolean>;
  restoreOcrText(itemId: string): Promise<boolean>;
  parseOcr(itemId: string): Promise<boolean>;
  parseQRCode(itemId: string): Promise<boolean>;
  generateQRCode(itemId: string): Promise<boolean>;
  deleteItem(itemId: string): Promise<boolean>;
  clearHistory(): Promise<boolean>;
  togglePin(itemId: string): Promise<boolean>;
  toggleFavorite(itemId: string): Promise<boolean>;
}

// ==================== Tags 操作类型 ====================

export interface TagOperations {
  getAll(): Promise<TagGroup[]>;
  create(name: string, color: string, description?: string): Promise<string>;
  update(
    tagId: string,
    updates: { name?: string; color?: string; description?: string }
  ): Promise<boolean>;
  delete(tagId: string): Promise<boolean>;
  bindToRecord(contentId: string, tagId: string): Promise<boolean>;
  unbindFromRecord(contentId: string, tagId: string): Promise<boolean>;
}

// ==================== App 操作类型 ====================

export interface AppOperations {
  restart(): Promise<boolean>;
  minimizeWindow(): Promise<boolean>;
  closeWindow(): Promise<boolean>;
}

export interface AppCacheStats {
  totalApps: number;
  cacheSize: number;
}

// ==================== 渲染进程 API 类型 ====================

export interface RendererAPI {
  store: StoreOperations;
  clipboard: ClipboardOperations;
  tags: TagOperations;
  app: AppOperations;
}

