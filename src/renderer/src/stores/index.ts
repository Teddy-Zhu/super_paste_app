import { createPinia } from 'pinia'
import { createPersistedState } from 'pinia-plugin-persistedstate'
import { createFieldChangePlugin } from './fieldChangePlugin'
import { sendSync, invoke, onSimple as on } from '@renderer/api/ipc'
import { isProxy, toRaw } from 'vue'
const store = createPinia()
// pinia持久化
store.use(
  createPersistedState({
    storage: {
      setItem: () => {
        // 写入由字段变更插件负责；此适配器只提供初始化读取。
      },
      getItem: (key: string) => {
        // 需要同步方法
        const val = sendSync('store-get-on', key)
        return JSON.stringify(val)
      }
    }
  })
)
store.use(
  createFieldChangePlugin(({ storeId, key, newValue }) => {
    invoke('store-set', `${storeId}.${key}`, isProxy(newValue) ? toRaw(newValue) : newValue)
  })
)

store.use(({ store: activeStore }) => {
  const unsubscribe = on(
    'store-value-changed',
    ({ key, value }: { key: string; value: unknown }) => {
      const [storeId, ...fieldParts] = key.split('.')
      if (activeStore.$id === storeId && fieldParts.length === 1) {
        activeStore.$setSilent(fieldParts[0], value)
      }
    }
  )
  const dispose = activeStore.$dispose.bind(activeStore)
  activeStore.$dispose = () => {
    unsubscribe()
    dispose()
  }
})

export default store
