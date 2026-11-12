import {
  on,
  onSimple,
  once,
  onceSimple,
  send,
  invoke,
  removeListener,
  removeAllListeners
} from './ipc'
import type { RendererAPI } from './types'
import { clipboard } from './clipboard'
import { tags } from './tags'
import { apps } from './apps'
import { app } from './app'

export { clipboard, tags, apps, app }

/**
 * 完整的渲染进程 API
 */
export const rendererAPI: RendererAPI = {
  // 剪贴板操作
  clipboard,

  // 标签操作
  tags,

  // 应用信息操作
  apps,

  // 应用控制操作
  app,

  // 原始 IPC 方法
  ipc: {
    on,
    onSimple,
    once,
    onceSimple,
    send,
    invoke,
    removeListener,
    removeAllListeners
  }
}

// 默认导出
export default rendererAPI
