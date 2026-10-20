import { watch, toRefs } from 'vue'
import type { PiniaPluginContext } from 'pinia'

declare module 'pinia' {
  export interface PiniaCustomProperties {
    $setSilent(key: string, value: unknown): void
  }
}

type FieldChangeCallback = (params: {
  storeId: string
  key: string
  oldValue: unknown
  newValue: unknown
}) => void

export function createFieldChangePlugin(callback: FieldChangeCallback) {
  return (context: PiniaPluginContext) => {
    const { store } = context
    const stateRefs = toRefs(store.$state)
    let silent = false

    // 注入方法：store.$setSilent(key, value)
    store.$setSilent = (key: string, value: unknown) => {
      if (key in store.$state) {
        silent = true
        try {
          store.$state[key] = value
        } finally {
          silent = false
        }
      } else {
        console.warn(`[fieldChangePlugin] key "${key}" 不存在于 store "${store.$id}" 中`)
      }
    }

    if (!store.$persist) {
      return
    }
    for (const key in stateRefs) {
      let oldVal = stateRefs[key].value
      watch(
        stateRefs[key],
        (newVal) => {
          if (silent) {
            oldVal = newVal
            return // 👈 跳过本次回调
          }

          callback({
            storeId: store.$id,
            key,
            oldValue: oldVal,
            newValue: newVal
          })
          oldVal = newVal
        },
        {
          deep: true,
          flush: 'sync'
        }
      )
    }
  }
}
