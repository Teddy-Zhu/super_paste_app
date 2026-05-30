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
