/**
 * 应用常量定义
 */

// ===== 窗口相关常量 =====
export const WINDOW_HEIGHT = 350 as const

// ===== 延迟时间常量 =====
export const SYSTEM_RESUME_DELAY = 2000 as const
export const MAX_KEEP_TIME_DEBOUNCE_DELAY = 2000 as const
export const MAX_HISTORY_SIZE_DEBOUNCE_DELAY = 800 as const

// ===== 窗口尺寸配置 =====
export const MAIN_WINDOW_CONFIG = {
  WIDTH_RATIO: 0.7,
  HEIGHT_RATIO: 0.75,
  MIN_WIDTH: 800,
  MAX_WIDTH: 1200,
  MIN_HEIGHT: 600,
  MAX_HEIGHT: 900
} as const

// ===== 剪贴板管理器配置 =====
export const CLIPBOARD_CONFIG = {
  CHECK_INTERVAL: 200,
  ENABLE_AUTO_SAVE: true
} as const
