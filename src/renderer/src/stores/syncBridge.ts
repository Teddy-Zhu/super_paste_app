import type { PiniaPluginContext } from 'pinia'
import { cloneStateValue, type FieldChange } from './fieldChangePlugin'
import type {
  StoreSnapshot,
  StoreValueChange,
  StoreWriteMetadata,
  StoreWriteResult
} from '@shared/store-sync'

interface SyncTransport {
  write(
    storeId: string,
    patch: Record<string, unknown>,
    metadata: StoreWriteMetadata
  ): Promise<StoreWriteResult>
  snapshot(storeId: string): StoreSnapshot
  subscribe(listener: (change: StoreValueChange) => void): () => void
}

interface StoreSyncBridge {
  onChange(change: FieldChange): void
  install(context: PiniaPluginContext): void
}

export function createStoreSyncBridge(transport: SyncTransport, clientId: string): StoreSyncBridge {
  const stores = new Map<
    string,
    {
      change(change: FieldChange): void
      receive(change: StoreValueChange): void
    }
  >()
  let unsubscribe: (() => void) | undefined
  let sequence = 0
  const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b)

  return {
    onChange(change: FieldChange): void {
      stores.get(change.storeId)?.change(change)
    },
    install({ store }: PiniaPluginContext): void {
      if (!store.$persist) return
      const initial = transport.snapshot(store.$id)
      const confirmed = new Map(Object.entries(cloneStateValue(store.$state)))
      const versions = new Map(Object.keys(store.$state).map((key) => [key, -1]))
      const pending = new Map<string, unknown>()
      const active = new Set<string>()
      const waiters = new Set<{ resolve(): void; reject(error: Error): void }>()
      let cycleError: Error | undefined
      let scheduled = false
      let disposed = false

      const updateStatus = (): void => {
        store.$syncStatus.pending = pending.size + active.size
      }
      const applyConfirmed = (key: string): void => {
        if (!pending.has(key)) store.$patchFromSync({ [key]: confirmed.get(key) })
      }
      const acceptValue = (key: string, value: unknown, version: number): void => {
        if (!versions.has(key) || version <= versions.get(key)!) return
        versions.set(key, version)
        confirmed.set(key, cloneStateValue(value))
        if (!active.has(key)) applyConfirmed(key)
      }
      const acceptSnapshot = (snapshot: StoreSnapshot): void => {
        if (disposed) return
        for (const key of versions.keys()) acceptValue(key, snapshot.value?.[key], snapshot.version)
      }
      // 首次 hydrate 与订阅之间可能有后台修改，以同一版本的完整快照收敛。
      acceptSnapshot(initial)

      const settle = (): void => {
        if (pending.size || active.size) return
        for (const waiter of waiters) {
          if (cycleError) waiter.reject(cycleError)
          else waiter.resolve()
        }
        waiters.clear()
      }
      store.$whenSynced = () => {
        if (disposed) return Promise.reject(new Error('配置存储已销毁'))
        if (!pending.size && !active.size) {
          return store.$syncStatus.error
            ? Promise.reject(new Error(store.$syncStatus.error))
            : Promise.resolve()
        }
        return new Promise<void>((resolve, reject) => waiters.add({ resolve, reject }))
      }

      const schedule = (): void => {
        if (scheduled || active.size > 0 || disposed) return
        scheduled = true
        queueMicrotask(() => {
          scheduled = false
          void flush()
        })
      }
      const flush = async (): Promise<void> => {
        if (disposed || active.size > 0) return
        const patch: Record<string, unknown> = {}
        for (const [key, value] of pending) {
          if (same(value, confirmed.get(key))) continue
          patch[key] = value
          active.add(key)
        }
        const keys = [...pending.keys()]
        pending.clear()
        if (!active.size) {
          keys.forEach(applyConfirmed)
          updateStatus()
          settle()
          return
        }
        updateStatus()
        const mutationId = `${clientId}:${++sequence}`
        try {
          const result = await transport.write(store.$id, patch, {
            clientId,
            mutationId
          })
          if (disposed) return
          if (!result.success) throw new Error(result.error || '配置保存失败')
          if (result.snapshot) acceptSnapshot(result.snapshot)
          else throw new Error('配置保存响应缺少快照')
          if (!cycleError) store.$syncStatus.error = null
        } catch (error) {
          if (disposed) return
          cycleError ??= error instanceof Error ? error : new Error(String(error))
          store.$syncStatus.error = cycleError.message
          // 失败时读取权威值；不回滚覆盖等待发送的较新本地修改。
          try {
            acceptSnapshot(transport.snapshot(store.$id))
          } catch (snapshotError) {
            console.error('[配置同步] 读取服务端配置失败:', snapshotError)
          }
        } finally {
          active.clear()
          if (!disposed) {
            keys.forEach(applyConfirmed)
            updateStatus()
            settle()
            if (pending.size) schedule()
          }
        }
      }
      stores.set(store.$id, {
        change({ key, newValue }) {
          if (!pending.size && !active.size) cycleError = undefined
          pending.set(key, cloneStateValue(newValue))
          updateStatus()
          schedule()
        },
        receive(change) {
          const field = change.key.slice(store.$id.length + 1)
          acceptValue(field, change.value, change.version)
        }
      })
      unsubscribe ??= transport.subscribe((change) => {
        stores.get(change.key.split('.')[0])?.receive(change)
      })
      const dispose = store.$dispose.bind(store)
      store.$dispose = () => {
        disposed = true
        pending.clear()
        for (const waiter of waiters) waiter.reject(new Error('配置存储已销毁'))
        waiters.clear()
        stores.delete(store.$id)
        if (!stores.size) {
          unsubscribe?.()
          unsubscribe = undefined
        }
        dispose()
      }
    }
  }
}
