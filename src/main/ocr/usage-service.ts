import { app } from 'electron'
import { join } from 'node:path'
import { AIOCRUsageStore } from './usage-store'
import { NotificationService } from '../services/notification-service'
import logger from '../utils/logger'
import type { AIOCRUsageEntry } from '../../shared/ai-ocr'

let usageStore: AIOCRUsageStore | undefined

export function getAIOCRUsageStore(): AIOCRUsageStore {
  if (!usageStore) {
    usageStore = new AIOCRUsageStore(join(app.getPath('userData'), 'ai-ocr-usage.db'), {
      onError: (error) => logger.error('[AI OCR 用量] 延迟落库失败，将重试:', error)
    })
  }
  return usageStore
}

export function recordAIOCRUsage(entry: AIOCRUsageEntry): void {
  getAIOCRUsageStore().record(entry)
  NotificationService.notifyFrontend('ocr:usage-updated', undefined)
}

export function closeAIOCRUsage(): void {
  usageStore?.close()
  usageStore = undefined
}
