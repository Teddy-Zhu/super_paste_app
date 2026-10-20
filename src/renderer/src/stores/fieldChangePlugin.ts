import { watch, reactive } from 'vue'
import type { PiniaPluginContext } from 'pinia'
import type { StoreChangeSource } from '@shared/store-sync'

export interface StoreSyncStatus {
  pending: number
  error: string | null
}

declare module 'pinia' {
  export interface PiniaCustomProperties {
    $patchFromSync(patch: Record<string, unknown>): void
    // 普通修改自动同步；只有需要确认保存结果时才等待。
    $whenSynced(): Promise<void>
    $syncStatus: StoreSyncStatus
  }
}

export interface FieldChange {
  storeId: string
  key: string
  oldValue: unknown
  newValue: unknown
  source: StoreChangeSource
}

// 配置按 JSON 保存；同时隔离响应式引用和后续嵌套修改。
export function cloneStateValue<T>(value: T): T {
  return value === undefined ? value : JSON.parse(JSON.stringify(value))
}

export function createFieldChangePlugin(
  callback: (change: FieldChange) => void,
  source: StoreChangeSource
) {
  return ({ store, options }: PiniaPluginContext): void => {
    if (!store.$persist) return
    let remoteDepth = 0
    const defaults = options.state?.() ?? {}
    const stops: (() => void)[] = []
    store.$syncStatus = reactive({ pending: 0, error: null })
    store.$patchFromSync = (patch) => {
      remoteDepth++
      try {
        store.$patch((state) => {
          for (const [key, value] of Object.entries(patch)) {
            if (Object.prototype.hasOwnProperty.call(state, key))
              state[key] = cloneStateValue(value === undefined ? defaults[key] : value)
          }
        })
      } finally {
        remoteDepth--
      }
    }
    for (const key of Object.keys(store.$state)) {
      let previous = JSON.stringify(store.$state[key])
      stops.push(
        watch(
          () => store.$state[key],
          (value) => {
            const next = JSON.stringify(value)
            const oldValue = previous
            previous = next
            if (remoteDepth || next === oldValue) return
            callback({
              storeId: store.$id,
              key,
              oldValue: oldValue === undefined ? undefined : JSON.parse(oldValue),
              newValue: next === undefined ? undefined : JSON.parse(next),
              source
            })
          },
          { deep: true, flush: 'sync' }
        )
      )
    }
    const dispose = store.$dispose.bind(store)
    store.$dispose = () => {
      stops.forEach((stop) => stop())
      dispose()
    }
  }
}
