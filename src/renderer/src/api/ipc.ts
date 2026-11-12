import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen, type Event, type UnlistenFn } from '@tauri-apps/api/event'
import {
  CREDENTIAL_FIELD_PATHS,
  type ConfigReadResult,
  type StoreValueChange
} from '@shared/store-sync'
import type { IpcRendererEvent, IpcRendererListener, IpcResponses } from './types'

type SimpleListener<Args extends unknown[]> = (...args: Args) => void
type AnyListener = (event: IpcRendererEvent, ...args: unknown[]) => void

const COMMANDS: Record<string, string> = {
  'clipboard-copy-color': 'clipboard_copy_color',
  'clipboard-get-history': 'clipboard_get_history',
  'clipboard-restore': 'clipboard_restore',
  'clipboard-restore-as-text': 'clipboard_restore_as_text',
  'clipboard-restore-ocr-text': 'clipboard_restore_ocr_text',
  'clipboard-parse-ocr': 'clipboard_parse_ocr',
  'clipboard-parse-qrcode': 'clipboard_parse_qrcode',
  'clipboard-generate-qrcode': 'clipboard_generate_qrcode',
  'clipboard-delete-item': 'clipboard_delete_item',
  'clipboard-clear-history': 'clipboard_clear_history',
  'clipboard-get-status': 'clipboard_get_status',
  'clipboard-start-listening': 'clipboard_start_listening',
  'clipboard-stop-listening': 'clipboard_stop_listening',
  'clipboard-update-config': 'clipboard_update_config',
  'get-all-tags': 'get_all_tags',
  'create-tag': 'create_tag',
  'update-tag': 'update_tag',
  'delete-tag': 'delete_tag',
  'get-tag-stats': 'get_tag_stats',
  'bind-tag-to-record': 'bind_tag_to_record',
  'unbind-tag-from-record': 'unbind_tag_from_record',
  'statistics:get-copy': 'statistics_get_copy',
  'statistics:clear-copy': 'statistics_clear_copy',
  'ocr:list-models': 'ocr_list_models',
  'ocr:download-model': 'ocr_download_model',
  'ocr:cancel-download': 'ocr_cancel_download',
  'ocr:delete-model': 'ocr_delete_model',
  'ocr:redownload-model': 'ocr_redownload_model',
  'ocr:ai-list-models': 'ocr_ai_list_models',
  'ocr:ai-test-connection': 'ocr_ai_test_connection',
  'ocr:get-usage': 'ocr_get_usage',
  'ocr:clear-usage': 'ocr_clear_usage',
  'mcp:get-state': 'mcp_get_state',
  'mcp:save-settings': 'mcp_save_settings',
  'mcp:generate-key': 'mcp_generate_key',
  'scheduled-sync:test-connection': 'scheduled_sync_test_connection',
  'scheduled-sync:manual-sync': 'scheduled_sync_manual_sync',
  'scheduled-sync:manual-sync-with-config': 'scheduled_sync_manual_sync_with_config',
  'scheduled-sync:get-status': 'scheduled_sync_get_status',
  'system-access-status': 'system_access_status',
  'system-access-retry': 'system_access_retry',
  'app:restart': 'app_restart',
  'app:window-minimize': 'app_window_minimize',
  'app:window-close': 'app_window_close',
  'export-diagnostics': 'export_diagnostics',
  'open-overlay-window': 'open_overlay_window',
  'close-overlay-window': 'close_overlay_window',
  'open-main-window': 'open_main_window',
  'open-external-url': 'open_external_url',
  'overlay-ready': 'overlay_ready',
  'overlay-open-prepared': 'overlay_open_prepared',
  'overlay-close-started': 'overlay_close_started',
  'open-overlay-finished': 'open_overlay_finished',
  'overlay-trace': 'overlay_trace',
  'store:get-snapshot': 'store_get_snapshot',
  'store:patch': 'store_patch',
  desktop_bootstrap: 'desktop_bootstrap'
}

const EVENT_CHANNELS = [
  'store-value-changed',
  'trigger-close-window',
  'prepare-overlay-open',
  'overlay-hidden',
  'overlay-modifiers-changed',
  'trigger-slide-in-animation',
  'overlay-position-updated',
  'clipboard-history-updated',
  'tags-updated',
  'record-ocr-updated',
  'ocr:task-status',
  'ocr:model-status',
  'ocr:usage-updated',
  'statistics:copy-updated',
  'mcp:status',
  'sync-status-updated',
  'sync-requires-restart'
] as const

const listeners = new Map<string, Map<object, Set<AnyListener>>>()
const configCache = new Map<string, ConfigReadResult>()
const pendingStoreChanges: StoreValueChange[] = []
let eventBridgeReady: Promise<void> | undefined
let bootstrapReady = false

function commandFor(channel: string): string {
  const command = COMMANDS[channel]
  if (!command) throw new Error(`不支持的 IPC 通道: ${channel}`)
  return command
}

function eventFor(channel: string): void {
  if (!EVENT_CHANNELS.includes(channel as (typeof EVENT_CHANNELS)[number])) {
    throw new Error(`不支持的 IPC 事件: ${channel}`)
  }
}

function clone<T>(value: T): T {
  return value === undefined ? value : JSON.parse(JSON.stringify(value))
}

function getPath(value: unknown, path: string[]): unknown {
  let current = value
  for (const part of path) {
    if (!current || typeof current !== 'object') return undefined
    current = (current as Record<string, unknown>)[part]
  }
  return current
}

function setPath(value: Record<string, unknown>, path: string[], nextValue: unknown): void {
  let current = value
  for (const part of path.slice(0, -1)) {
    const child = current[part]
    if (!child || typeof child !== 'object' || Array.isArray(child)) current[part] = {}
    current = current[part] as Record<string, unknown>
  }
  current[path[path.length - 1]] = clone(nextValue)
}

function deletePath(value: Record<string, unknown>, path: string[]): void {
  let current: Record<string, unknown> = value
  for (const part of path.slice(0, -1)) {
    const child = current[part]
    if (!child || typeof child !== 'object' || Array.isArray(child)) return
    current = child as Record<string, unknown>
  }
  delete current[path[path.length - 1]]
}

function mergeProjectedField(
  storeId: string,
  field: string,
  current: unknown,
  incoming: unknown
): unknown {
  if (!incoming || typeof incoming !== 'object' || Array.isArray(incoming)) return clone(incoming)
  const projected = clone(incoming) as Record<string, unknown>
  for (const path of CREDENTIAL_FIELD_PATHS) {
    const prefix = `${storeId}.${field}.`
    if (!path.startsWith(prefix)) continue
    const nestedPath = path.slice(prefix.length).split('.')
    const existing = getPath(current, nestedPath)
    if (existing === undefined) deletePath(projected, nestedPath)
    else setPath(projected, nestedPath, existing)
  }
  return projected
}

function applyStoreChange(change: StoreValueChange): void {
  const separator = change.key.indexOf('.')
  if (separator < 1) return
  const storeId = change.key.slice(0, separator)
  const fieldPath = change.key.slice(separator + 1)
  const field = fieldPath.split('.')[0]
  const config = configCache.get(storeId)
  if (!config || !config.snapshot.value || !field) return
  const version = config.fieldVersions[change.key] ?? -1
  if (change.version <= version) return
  const current = config.snapshot.value[field]
  const fieldValue =
    fieldPath === field
      ? mergeProjectedField(storeId, field, current, change.value)
      : (() => {
          const next =
            current && typeof current === 'object' && !Array.isArray(current)
              ? clone(current as Record<string, unknown>)
              : {}
          setPath(next, fieldPath.slice(field.length + 1).split('.'), change.value)
          return mergeProjectedField(storeId, field, current, next)
        })()
  config.snapshot.value = { ...config.snapshot.value, [field]: fieldValue }
  config.snapshot.version = Math.max(config.snapshot.version, change.version)
  config.fieldVersions = { ...config.fieldVersions, [change.key]: change.version }
}

function registerListener(channel: string, listener: object, handler: AnyListener): () => void {
  eventFor(channel)
  let channelListeners = listeners.get(channel)
  if (!channelListeners) {
    channelListeners = new Map()
    listeners.set(channel, channelListeners)
  }
  let handlers = channelListeners.get(listener)
  if (!handlers) {
    handlers = new Set()
    channelListeners.set(listener, handlers)
  }
  handlers.add(handler)
  return () => {
    handlers?.delete(handler)
    if (!handlers?.size) channelListeners?.delete(listener)
    if (!channelListeners?.size) listeners.delete(channel)
  }
}

function dispatchEvent(channel: string, event: Event<unknown>): void {
  const args = Array.isArray(event.payload) ? event.payload : [event.payload]
  if (channel === 'store-value-changed') {
    const change = args[0] as StoreValueChange | undefined
    if (change) {
      if (bootstrapReady) applyStoreChange(change)
      else pendingStoreChanges.push(change)
    }
  }
  const ipcEvent: IpcRendererEvent = { channel, payload: args }
  for (const handlers of [...(listeners.get(channel)?.values() ?? [])]) {
    for (const handler of [...handlers]) handler(ipcEvent, ...args)
  }
}

export async function initializeIpcEvents(): Promise<void> {
  if (!eventBridgeReady) {
    eventBridgeReady = Promise.all(
      EVENT_CHANNELS.map(async (channel) => {
        const unlisten: UnlistenFn = await listen(channel, (event) => dispatchEvent(channel, event))
        return unlisten
      })
    ).then(() => undefined)
  }
  return eventBridgeReady
}

export function installDesktopConfig(config: Record<string, ConfigReadResult>): void {
  for (const [storeId, readResult] of Object.entries(config)) {
    const queued = pendingStoreChanges
      .filter((change) => change.key.startsWith(`${storeId}.`))
      .sort((a, b) => a.version - b.version)
    configCache.set(storeId, clone(readResult))
    for (const change of queued) applyStoreChange(change)
  }
  bootstrapReady = true
  pendingStoreChanges.length = 0
}

export function getConfigReadResult(storeId: string): ConfigReadResult {
  const result = configCache.get(storeId)
  if (!result) throw new Error(`配置尚未初始化: ${storeId}`)
  return clone(result)
}

export function updateConfigReadResult(storeId: string, result: ConfigReadResult): void {
  const current = configCache.get(storeId)
  if (!current) {
    configCache.set(storeId, clone(result))
    return
  }
  const next = clone(current)
  const values = { ...(next.snapshot.value ?? {}) }
  for (const [field, value] of Object.entries(result.snapshot.value ?? {})) {
    const path = `${storeId}.${field}`
    const incomingVersion = result.fieldVersions[path] ?? result.snapshot.version
    const currentVersion = next.fieldVersions[path] ?? -1
    if (incomingVersion < currentVersion) continue
    values[field] = mergeProjectedField(storeId, field, values[field], value)
    next.fieldVersions[path] = incomingVersion
  }
  next.snapshot = {
    value: values,
    version: Math.max(next.snapshot.version, result.snapshot.version)
  }
  const credentialFields = { ...next.credentialFields }
  const storePrefix = `${storeId}.`
  for (const [path, status] of Object.entries(result.credentialFields)) {
    if (!path.startsWith(storePrefix)) continue
    const field = path.slice(storePrefix.length).split('.')[0]
    if (!field) continue
    const versionPath = `${storeId}.${field}`
    const incomingVersion = result.fieldVersions[versionPath] ?? result.snapshot.version
    const currentVersion = current.fieldVersions[versionPath] ?? -1
    if (incomingVersion < currentVersion) continue
    credentialFields[path] = status
  }
  next.credentialFields = credentialFields
  configCache.set(storeId, next)
}

export function on<Args extends unknown[]>(
  channel: string,
  listener: IpcRendererListener<Args>
): () => void {
  return registerListener(channel, listener, listener as AnyListener)
}

export function onSimple<Args extends unknown[]>(
  channel: string,
  listener: SimpleListener<Args>
): () => void {
  const handler = (_event: IpcRendererEvent, ...args: unknown[]): void =>
    listener(...(args as Args))
  return registerListener(channel, listener, handler)
}

export function once<Args extends unknown[]>(
  channel: string,
  listener: IpcRendererListener<Args>
): () => void {
  let unsubscribe = (): void => {}
  const handler = (event: IpcRendererEvent, ...args: unknown[]): void => {
    unsubscribe()
    listener(event, ...(args as Args))
  }
  unsubscribe = registerListener(channel, listener, handler)
  return unsubscribe
}

export function onceSimple<Args extends unknown[]>(
  channel: string,
  listener: SimpleListener<Args>
): () => void {
  let unsubscribe = (): void => {}
  const handler = (_event: IpcRendererEvent, ...args: unknown[]): void => {
    unsubscribe()
    listener(...(args as Args))
  }
  unsubscribe = registerListener(channel, listener, handler)
  return unsubscribe
}

export function removeListener(channel: string, listener: AnyListener): void {
  const handlers = listeners.get(channel)?.get(listener)
  if (!handlers) return
  const channelListeners = listeners.get(channel)
  channelListeners?.delete(listener)
  if (!channelListeners?.size) listeners.delete(channel)
}

export function removeAllListeners(channel: string): void {
  eventFor(channel)
  listeners.delete(channel)
}

export function invoke<Channel extends keyof IpcResponses>(
  channel: Channel,
  ...args: unknown[]
): Promise<IpcResponses[Channel]>
export function invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>
export function invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T> {
  return tauriInvoke<T>(commandFor(channel), { args })
}

export function send(channel: string, ...args: unknown[]): void {
  void invoke(channel, ...args).catch((error) => {
    console.error(`[IPC] ${channel} 调用失败:`, error)
  })
}

export default {
  on,
  onSimple,
  once,
  onceSimple,
  removeListener,
  removeAllListeners,
  send,
  invoke
}
