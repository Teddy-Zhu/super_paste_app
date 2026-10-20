import type { ClipboardItem } from '@renderer/api/types'

// 错误类型
export class OverlayError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly context?: Record<string, unknown>
  ) {
    super(message)
    this.name = 'OverlayError'
  }
}

// 错误代码枚举
export enum OverlayErrorCode {
  ANIMATION_FAILED = 'ANIMATION_FAILED',
  SEARCH_FAILED = 'SEARCH_FAILED',
  CLIPBOARD_LOAD_FAILED = 'CLIPBOARD_LOAD_FAILED',
  TAG_LOAD_FAILED = 'TAG_LOAD_FAILED',
  INVALID_STATE = 'INVALID_STATE'
}

export function isClipboardItem(value: unknown): value is ClipboardItem {
  if (typeof value !== 'object' || value === null) return false
  const item = value as Record<string, unknown>
  return (
    typeof item.id === 'string' &&
    typeof item.content === 'object' &&
    item.content !== null &&
    typeof item.primaryType === 'string' &&
    Array.isArray(item.availableFormats) &&
    item.availableFormats.every((format) => typeof format === 'string') &&
    typeof item.timestamp === 'number' &&
    Number.isFinite(item.timestamp) &&
    typeof item.lastUsed === 'number' &&
    Number.isFinite(item.lastUsed)
  )
}
