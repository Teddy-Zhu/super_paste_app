import { createPinia } from 'pinia'
import { getConfigReadResult, invoke, updateConfigReadResult } from '@renderer/api/ipc'
import { createStoreSyncBridge } from './syncBridge'
import type { StoreWriteResult } from '@shared/store-sync'

const clientId = crypto.randomUUID()
const store = createPinia()
store.use(
  createStoreSyncBridge(
    {
      read: getConfigReadResult,
      update: updateConfigReadResult,
      write: (storeId, patch, metadata) =>
        invoke<StoreWriteResult>('store:patch', storeId, patch, metadata)
    },
    clientId
  )
)

export default store
