import { onUnmounted, ref, type Ref } from 'vue'
import type { ClipboardItem } from '@renderer/api/types'
import type { OCRTaskStatus } from '@shared/ai-ocr'

const ERROR_RETENTION_MS = 2 * 60000
const MAX_STATUSES = 500

export function useOcrStatuses(history: Ref<ClipboardItem[]>): {
  statuses: Ref<Record<string, OCRTaskStatus>>
  apply: (status: OCRTaskStatus) => void
  retain: (ids: Set<string>) => void
  start: () => void
} {
  const statuses = ref<Record<string, OCRTaskStatus>>({})
  const errorsExpireAt = new Map<string, number>()
  let expiryTimer: ReturnType<typeof setInterval> | undefined

  const remove = (id: string): void => {
    delete statuses.value[id]
    errorsExpireAt.delete(id)
  }
  const retain = (ids: Set<string>): void => {
    for (const id of Object.keys(statuses.value)) {
      if (!ids.has(id)) remove(id)
    }
  }
  const prune = (): void => {
    retain(new Set(history.value.map((item) => item.id)))
    for (const [id, expiresAt] of errorsExpireAt) {
      if (expiresAt <= Date.now()) remove(id)
    }
  }
  const apply = (status: OCRTaskStatus): void => {
    if (status.state === 'completed' || status.state === 'cancelled') {
      remove(status.contentId)
      return
    }
    if (status.state === 'error') {
      if (!history.value.some((item) => item.id === status.contentId)) return
      errorsExpireAt.set(status.contentId, Date.now() + ERROR_RETENTION_MS)
    } else {
      errorsExpireAt.delete(status.contentId)
    }
    statuses.value[status.contentId] = status
    const ids = Object.keys(statuses.value)
    for (const id of ids.slice(0, Math.max(0, ids.length - MAX_STATUSES))) remove(id)
  }
  const start = (): void => {
    clearInterval(expiryTimer)
    expiryTimer = setInterval(prune, 60000)
  }
  onUnmounted(() => clearInterval(expiryTimer))
  return { statuses, apply, retain, start }
}
