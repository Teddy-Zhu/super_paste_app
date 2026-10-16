import type { ClipboardItem } from '@renderer/api/types'

// 错误类型
export class OverlayError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly context?: Record<string, any>
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

export function isClipboardItem(value: any): value is ClipboardItem {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof value.id === 'string' &&
    typeof value.content === 'object' &&
    value.content !== null &&
    typeof value.primaryType === 'string' &&
    Array.isArray(value.availableFormats) &&
    typeof value.timestamp === 'number' &&
    typeof value.lastUsed === 'number'
  )
}
