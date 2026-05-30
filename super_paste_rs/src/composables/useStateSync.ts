/**
 * 前后端状态同步 composable
 * 使用 source 标识变更来源，避免循环更新
 */

import { onUnmounted } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

export interface StateUpdate {
  source: 'frontend' | 'backend'
  data: Record<string, any>
}

/**
 * 订阅后端状态变更并应用到 Pinia store
 * @param store - Pinia store 实例
 * @param eventKey - 监听的事件名（默认 'state-changed'）
 */
export function useStateSync(
  store: any,
  eventKey: string = 'state-changed'
) {
  let unlisten: (() => void) | null = null

  // 监听后端广播的状态变更
  listen<StateUpdate>(eventKey, (event) => {
    const { source, data } = event.payload

    // 只处理后端发起的变更，忽略前端自己发起的
    if (source !== 'backend') {
      return
    }

    console.log('[StateSync] 收到后端状态变更:', data)

    // 使用 $patch 更新 store（不触发 watcher 避免循环）
    if (store.$patch) {
      store.$patch(data)
    }
  }).then((fn) => {
    unlisten = fn
  })

  // 发送前端状态变更到后端
  const sendUpdate = (data: Record<string, any>) => {
    const update: StateUpdate = {
      source: 'frontend',
      data
    }
    invoke('sync_state_update', { update }).catch((err) => {
      console.error('[StateSync] 发送状态变更失败:', err)
    })
  }

  // 清理监听器
  onUnmounted(() => {
    if (unlisten) {
      unlisten()
      unlisten = null
    }
  })

  return { sendUpdate }
}

/**
 * 监听多个 store 的变更并同步到后端
 * @param stores - { eventName: store } 映射
 */
export function useMultiStoreSync(
  stores: Record<string, any>
) {
  const cleanupFns: (() => void)[] = []

  for (const [eventKey, store] of Object.entries(stores)) {
    const { sendUpdate } = useStateSync(store, eventKey)

    // 监听 store 变更并发送到后端
    const stopWatcher = store.$subscribe(
      (_mutation: any, state: any) => {
        sendUpdate(state)
      }
    )

    cleanupFns.push(stopWatcher)
  }

  onUnmounted(() => {
    cleanupFns.forEach((fn) => fn())
  })
}
