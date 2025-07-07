import { watch, toRefs } from 'vue'
import type { PiniaPluginContext } from 'pinia'

type FieldChangeCallback = (params: {
  storeId: string
  key: string
  oldValue: any
  newValue: any
}) => void

export function createFieldChangePlugin(callback: FieldChangeCallback) {
  return (context: PiniaPluginContext) => {
    const { store } = context
    const stateRefs = toRefs(store.$state)
    const silentFlags = new Set<string>()

    // 注入方法：store.$setSilent(key, value)
    store.$setSilent = (key: string, value: any) => {
      if (key in store.$state) {
        silentFlags.add(`${store.$id}.${key}`)
        store[key] = value
      } else {
        console.warn(`[fieldChangePlugin] key "${key}" 不存在于 store "${store.$id}" 中`)
      }
    }

    if (!store.$persist){
      console.log('persisted store, skip field change plugin', store.$id)
      return
    }
    for (const key in stateRefs) {
      let oldVal = stateRefs[key].value
      watch(stateRefs[key], (newVal) => {
        const flag = `${store.$id}.${key}`
        if (silentFlags.has(flag)) {
          silentFlags.delete(flag)
          oldVal = newVal
          console.log('skip field change', flag)
          return // 👈 跳过本次回调
        }

        callback({
          storeId: store.$id,
          key,
          oldValue: oldVal,
          newValue: newVal,
        })
        oldVal = newVal
      })
    }
  }
}
