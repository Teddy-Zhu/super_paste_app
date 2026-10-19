import type { ClipboardHistoryResult, ClipboardItem } from '@renderer/api/types'

// IPC 会复制对象和图片字节；按值比较并复用旧引用，避免无变化时触发渲染和图片解码。
function reuseUnchanged(previous: unknown, incoming: unknown): unknown {
  if (previous === incoming) return previous

  if (previous instanceof Uint8Array || incoming instanceof Uint8Array) {
    if (
      !(previous instanceof Uint8Array) ||
      !(incoming instanceof Uint8Array) ||
      previous.byteLength !== incoming.byteLength
    )
      return incoming
    for (let index = 0; index < incoming.byteLength; index++) {
      if (previous[index] !== incoming[index]) return incoming
    }
    return previous
  }

  if (Array.isArray(previous) || Array.isArray(incoming)) {
    if (!Array.isArray(previous) || !Array.isArray(incoming)) return incoming
    const merged = incoming.map((value, index) => reuseUnchanged(previous[index], value))
    return previous.length === merged.length &&
      merged.every((value, index) => value === previous[index])
      ? previous
      : merged
  }

  if (
    previous !== null &&
    incoming !== null &&
    typeof previous === 'object' &&
    typeof incoming === 'object'
  ) {
    const previousRecord = previous as Record<string, unknown>
    const incomingRecord = incoming as Record<string, unknown>
    const keys = Object.keys(incomingRecord)
    const merged = { ...incomingRecord }
    let unchanged = Object.keys(previousRecord).length === keys.length
    for (const key of keys) {
      merged[key] = reuseUnchanged(previousRecord[key], incomingRecord[key])
      if (
        !Object.prototype.hasOwnProperty.call(previousRecord, key) ||
        merged[key] !== previousRecord[key]
      ) {
        unchanged = false
      }
    }
    return unchanged ? previous : merged
  }

  return incoming
}

export function reconcileClipboardHistory(
  previous: ClipboardHistoryResult,
  incoming: ClipboardHistoryResult
): ClipboardHistoryResult {
  const previousById = new Map(previous.items.map((item) => [item.id, item]))
  const mergedItems = incoming.items.map(
    (item) => reuseUnchanged(previousById.get(item.id), item) as ClipboardItem
  )
  const items =
    previous.items.length === mergedItems.length &&
    mergedItems.every((item, index) => item === previous.items[index])
      ? previous.items
      : mergedItems

  return {
    items,
    apps: reuseUnchanged(previous.apps, incoming.apps) as ClipboardHistoryResult['apps']
  }
}
