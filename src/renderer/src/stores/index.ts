import { createPinia } from 'pinia'
import { createPersistedState } from 'pinia-plugin-persistedstate'
import { createFieldChangePlugin } from './fieldChangePlugin'
import { createStoreSyncBridge } from './syncBridge'
import { sendSync, invoke, onSimple } from '@renderer/api/ipc'
import type { StoreSnapshot, StoreValueChange, StoreWriteResult } from '@shared/store-sync'

const clientId = crypto.randomUUID()
const readSnapshot = (storeId: string): StoreSnapshot => {
  const result = sendSync('store:get-snapshot', storeId) as {
    success: boolean
    data?: StoreSnapshot
    error?: string
  }
  if (!result.success || !result.data) throw new Error(result.error || '读取配置失败')
  return result.data
}
const bridge = createStoreSyncBridge(
  {
    write: (storeId, patch, metadata) =>
      invoke<StoreWriteResult>('store:patch', storeId, patch, metadata),
    snapshot: readSnapshot,
    subscribe: (listener) =>
      onSimple('store-value-changed', (change: StoreValueChange) => listener(change))
  },
  clientId
)
const store = createPinia()
store.use(
  createPersistedState({
    storage: {
      setItem: () => {},
      getItem: (key) => JSON.stringify(readSnapshot(key).value) ?? null
    }
  })
)
store.use(createFieldChangePlugin(bridge.onChange, { kind: 'renderer', id: clientId }))
store.use(bridge.install)
export default store
