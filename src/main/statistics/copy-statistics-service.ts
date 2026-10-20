import { app } from 'electron'
import { join } from 'node:path'
import { CopyStatisticsStore } from './copy-statistics-store'
import type { CopyStatisticsEvent } from '../../shared/clipboard-statistics'
import { NotificationService } from '../services/notification-service'
import logger from '../utils/logger'

let statisticsStore: CopyStatisticsStore | undefined
let notificationTimer: ReturnType<typeof setTimeout> | undefined

export const getCopyStatisticsStore = (): CopyStatisticsStore => {
  return (statisticsStore ??= new CopyStatisticsStore(
    join(app.getPath('userData'), 'clipboard-statistics.db'),
    { onError: (error) => logger.error('[复制统计] 延迟落库失败，将重试:', error) }
  ))
}

export const recordCopyStatistics = (event: CopyStatisticsEvent): void => {
  if (!getCopyStatisticsStore().record(event) || notificationTimer) return
  // UI 更新也合并处理，不随每条复制事件重新查询聚合。
  notificationTimer = setTimeout(() => {
    notificationTimer = undefined
    NotificationService.notifyFrontend('statistics:copy-updated', undefined)
  }, 1000)
  notificationTimer.unref()
}

export const closeCopyStatistics = (): void => {
  if (notificationTimer) clearTimeout(notificationTimer)
  notificationTimer = undefined
  statisticsStore?.close()
  statisticsStore = undefined
}
