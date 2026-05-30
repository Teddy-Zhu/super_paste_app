/**
 * IPC 通信层 - Tauri 适配器
 * 提供与原 Electron 项目相同的 API 接口
 */

import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, once as tauriOnce, emit as tauriEmit } from '@tauri-apps/api/event'

type SimpleListener = (...args: any[]) => void
type IpcRendererListener = (event: any, ...args: any[]) => void

// Electron channel → Tauri command 映射
const CHANNEL_TO_COMMAND: Record<string, string> = {
  'store-get': 'store_get',
  'store-set': 'store_set',
  'store-get-on': 'store_get_value',
  'clipboard-get-history': 'get_clipboard_history',
  'clipboard-restore': 'restore_clipboard_item',
  'clipboard-restore-as-text': 'restore_clipboard_as_text',
  'clipboard-restore-ocr-text': 'restore_clipboard_ocr_text',
  'clipboard-parse-ocr': 'parse_clipboard_ocr',
  'clipboard-parse-qrcode': 'parse_clipboard_qrcode',
  'clipboard-generate-qrcode': 'generate_clipboard_qrcode',
  'clipboard-delete-item': 'delete_clipboard_item',
  'clipboard-clear-history': 'clear_clipboard_history',
  'clipboard-get-status': 'get_clipboard_status',
  'clipboard-start-listening': 'start_clipboard_listening',
  'clipboard-stop-listening': 'stop_clipboard_listening',
  'clipboard-update-config': 'update_clipboard_config',
  'get-all-tags': 'get_all_tags',
  'create-tag': 'create_tag',
  'update-tag': 'update_tag',
  'delete-tag': 'delete_tag',
  'get-tag-stats': 'get_tag_stats',
  'bind-tag-to-record': 'bind_tag_to_record',
  'unbind-tag-from-record': 'unbind_tag_from_record',
  'get-all-apps': 'get_all_apps',
  'get-app-cache-stats': 'get_app_cache_stats',
  'app:restart': 'restart_app',
  'app:window-minimize': 'minimize_window',
  'app:window-close': 'close_window',
}

const listenerCache = new Map<string, Set<() => void>>()

export function on(channel: string, listener: IpcRendererListener): () => void {
  const unlistenPromise = tauriListen(channel, (event) => {
    listener(event, event.payload)
  })

  const unlisten = () => {
    unlistenPromise.then((fn) => fn())
  }

  if (!listenerCache.has(channel)) {
    listenerCache.set(channel, new Set())
  }
  listenerCache.get(channel)!.add(unlisten)

  return unlisten
}

export function onSimple(channel: string, listener: SimpleListener): () => void {
  const unlistenPromise = tauriListen(channel, (event) => {
    listener(event.payload)
  })

  const unlisten = () => {
    unlistenPromise.then((fn) => fn())
  }

  if (!listenerCache.has(channel)) {
    listenerCache.set(channel, new Set())
  }
  listenerCache.get(channel)!.add(unlisten)

  return unlisten
}

export function once(channel: string, listener: IpcRendererListener): () => void {
  const unlistenPromise = tauriOnce(channel, (event) => {
    listener(event, event.payload)
  })
  return () => {
    unlistenPromise.then((fn) => fn())
  }
}

export function onceSimple(channel: string, listener: SimpleListener): () => void {
  const unlistenPromise = tauriOnce(channel, (event) => {
    listener(event.payload)
  })
  return () => {
    unlistenPromise.then((fn) => fn())
  }
}

export function removeListener(channel: string, listener: (...args: any[]) => void): void {
  console.warn('removeListener: 使用 on() 返回的取消函数')
}

export function removeAllListeners(channel: string): void {
  const listeners = listenerCache.get(channel)
  if (listeners) {
    listeners.forEach((unlisten) => unlisten())
    listenerCache.delete(channel)
  }
}

export function send(channel: string, ...args: any[]): void {
  tauriEmit(channel, args.length === 1 ? args[0] : args)
}

export function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
  const command = CHANNEL_TO_COMMAND[channel] || channel
  const invokeArgs = args.length === 0 ? undefined : args.length === 1 ? args[0] : args
  return tauriInvoke<T>(command, invokeArgs)
}

export function sendSync<T = any>(channel: string, ...args: any[]): T {
  // Tauri 不支持同步 IPC，用 localStorage 做 fallback
  const key = typeof args[0] === 'string' ? args[0] : ''
  const storageKey = `store:${key}`
  try {
    const stored = localStorage.getItem(storageKey)
    if (stored !== null) return JSON.parse(stored)
  } catch {}
  return {} as T
}

export default { on, onSimple, once, onceSimple, removeListener, removeAllListeners, send, invoke, sendSync }
