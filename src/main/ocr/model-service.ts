import { app, net } from 'electron'
import { join } from 'node:path'
import { OCRModelStore } from './model-store'
import { NotificationService } from '../services/notification-service'

let modelStore: OCRModelStore | undefined

export function getOCRModelStore(): OCRModelStore {
  if (!modelStore) {
    modelStore = new OCRModelStore({
      root: join(app.getPath('userData'), 'ocr-models'),
      fetch: net.fetch,
      onStatus: (status) => NotificationService.notifyFrontend('ocr:model-status', status)
    })
    app.once('before-quit', () => modelStore?.cancelAll())
  }
  return modelStore
}
