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
      setItem: (key: string, _: string) => {
        // 这里我使用异步方法保存数据
        // electronStore.setStoreValue(key, value)
        console.log('setItem', key)
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
  createFieldChangePlugin(({ storeId, key, oldValue, newValue }) => {
    console.log(`[${storeId}] 字段 ${key} 变了:`, oldValue, '→', newValue)
    // 也可以上传到服务器、日志记录等

    invoke('store-set', `${storeId}.${key}`, isProxy(newValue) ? toRaw(newValue) : newValue)
  })
)

on('store-value-changed', ({ key, value }) => {
  const parsedKey = key.split('.')
  const storeId = parsedKey[0]
  const fieldKey = parsedKey[1]

  //@ts-ignore
  const stores = Array.from(store._s.values())
  stores.forEach((store: any) => {
    if (store.$id === storeId) {
      console.log('store-value-changed set', storeId, fieldKey, value)
      store.$setSilent(fieldKey, value)
    }
  })
  console.log('store-value-changed', key, value)
})

export default store
