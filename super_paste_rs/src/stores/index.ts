import { createPinia } from 'pinia'
import { createPersistedState } from 'pinia-plugin-persistedstate'

const store = createPinia()

// 使用 localStorage 作为持久化存储（Tauri 不支持同步 IPC）
store.use(
  createPersistedState({
    storage: localStorage
  })
)

export default store
