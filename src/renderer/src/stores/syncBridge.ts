import type { PiniaPluginContext, StateTree } from 'pinia'
import { reactive } from 'vue'
import { onSimple } from '@renderer/api/ipc'
import {
  CREDENTIAL_FIELD_PATHS,
  type ConfigReadResult,
  type CredentialChanges,
  type CredentialFieldPath,
  type StoreValueChange,
  type StoreWriteMetadata,
  type StoreWriteResult
} from '@shared/store-sync'

export interface StoreCommitOptions {
  credentialChanges?: CredentialChanges
}

export interface StoreSyncStatus {
  pending: number
  error: string | null
}

declare module 'pinia' {
  export interface PiniaCustomProperties {
    $commit(patch: object, options?: StoreCommitOptions): Promise<void>
    $whenSynced(): Promise<void>
    $syncStatus: StoreSyncStatus
  }
}

interface SyncTransport {
  read(storeId: string): ConfigReadResult
  update(storeId: string, result: ConfigReadResult): void
  write(
    storeId: string,
    patch: Record<string, unknown>,
    metadata: StoreWriteMetadata
  ): Promise<StoreWriteResult>
}

const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b)

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

function pruneEmptyObjects(value: Record<string, unknown>): void {
  for (const [key, child] of Object.entries(value)) {
    if (!child || typeof child !== 'object' || Array.isArray(child)) continue
    pruneEmptyObjects(child as Record<string, unknown>)
    if (Object.keys(child).length === 0) delete value[key]
  }
}

function withoutCredentials(storeId: string, field: string, value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const projected = clone(value) as Record<string, unknown>
  for (const credentialPath of CREDENTIAL_FIELD_PATHS) {
    const prefix = `${storeId}.${field}.`
    if (credentialPath.startsWith(prefix)) {
      deletePath(projected, credentialPath.slice(prefix.length).split('.'))
    }
  }
  pruneEmptyObjects(projected)
  return projected
}

function credentialPathForStore(storeId: string, path: CredentialFieldPath): string[] | null {
  const prefix = `${storeId}.`
  return path.startsWith(prefix) ? path.slice(prefix.length).split('.') : null
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function mergeStatePatch(state: StateTree, patch: Record<string, unknown>): void {
  const target = state as Record<string, unknown>
  for (const [key, value] of Object.entries(patch)) {
    if (isPlainObject(value) && isPlainObject(target[key])) {
      mergeStatePatch(target[key] as StateTree, value)
    } else {
      target[key] = clone(value)
    }
  }
}

export function createStoreSyncBridge(transport: SyncTransport, clientId: string) {
  let sequence = 0
  return ({ store }: PiniaPluginContext): void => {
    if (store.$id !== 'settings' && store.$id !== 'sync') return

    const storeId = store.$id
    const originalPatch = store.$patch.bind(store)
    const rawPatch = (patch: Record<string, unknown>): void => {
      originalPatch((state) => mergeStatePatch(state, patch))
    }
    const initial = transport.read(storeId)
    if (initial.snapshot.value) {
      const hydrated = clone(initial.snapshot.value)
      for (const credentialPath of CREDENTIAL_FIELD_PATHS) {
        const relativePath = credentialPathForStore(storeId, credentialPath)
        if (!relativePath || getPath(hydrated, relativePath) !== undefined) continue
        const fallback = getPath(store.$state, relativePath)
        if (fallback !== undefined) setPath(hydrated, relativePath, fallback)
      }
      rawPatch(hydrated)
    }

    const latestMutation = new Map<string, string>()
    const pendingFields = new Set<string>()
    const defaultState = clone(store.$state) as Record<string, unknown>
    let queue: Promise<void> = Promise.resolve()
    let latestOperation: Promise<void> = Promise.resolve()
    let latestError: Error | null = null
    const listeners: (() => void)[] = []
    store.$syncStatus = reactive({ pending: 0, error: null })

    const setAcceptedFields = (result: ConfigReadResult, keys: string[]): void => {
      if (!result.snapshot.value) return
      const accepted: Record<string, unknown> = {}
      for (const key of keys) {
        if (Object.prototype.hasOwnProperty.call(result.snapshot.value, key)) {
          accepted[key] = result.snapshot.value[key]
        } else {
          accepted[key] = defaultState[key]
        }
      }
      if (Object.keys(accepted).length) rawPatch(accepted)
    }

    const createPatch = (
      patch: Record<string, unknown>,
      options: StoreCommitOptions
    ): { localPatch: Record<string, unknown>; commandPatch: Record<string, unknown> } => {
      const localPatch = clone(patch)
      const commandPatch = clone(patch)
      const credentialChanges = options.credentialChanges ?? {}

      for (const credentialPath of CREDENTIAL_FIELD_PATHS) {
        const relativePath = credentialPathForStore(storeId, credentialPath)
        if (!relativePath) continue
        const intent = credentialChanges[credentialPath]
        const status = transport.read(storeId).credentialFields[credentialPath]
        if (intent && status === 'unavailable') {
          throw new Error('当前窗口无权修改此凭据')
        }

        if (intent === 'set') {
          const value = getPath(patch, relativePath)
          if (typeof value !== 'string' || !value.trim()) {
            throw new Error(`凭据设为可用时必须提供非空值: ${credentialPath}`)
          }
          continue
        }

        if (intent === 'unset') {
          deletePath(commandPatch, relativePath)
          setPath(localPatch, relativePath, '')
          continue
        }

        deletePath(commandPatch, relativePath)
        const current = getPath(transport.read(storeId).snapshot.value, relativePath)
        if (current === undefined) deletePath(localPatch, relativePath)
        else setPath(localPatch, relativePath, current)
      }

      pruneEmptyObjects(commandPatch)
      return { localPatch, commandPatch }
    }

    const submit = (
      patch: Record<string, unknown>,
      options: StoreCommitOptions = {}
    ): Promise<void> => {
      const { localPatch, commandPatch } = createPatch(patch, options)
      rawPatch(localPatch)
      const credentialChanges = options.credentialChanges ?? {}
      const accepted = transport.read(storeId)
      const credentialSetFields = new Set(
        Object.entries(credentialChanges)
          .filter(([, intent]) => intent === 'set')
          .map(([path]) => credentialPathForStore(storeId, path as CredentialFieldPath)?.[0])
          .filter((field): field is string => Boolean(field))
      )
      const fields = Object.keys(commandPatch).filter(
        (field) =>
          credentialSetFields.has(field) ||
          !same(
            withoutCredentials(storeId, field, commandPatch[field]),
            withoutCredentials(storeId, field, accepted.snapshot.value?.[field])
          )
      )
      const wirePatch = Object.fromEntries(fields.map((field) => [field, commandPatch[field]]))
      if (!fields.length && !Object.keys(credentialChanges).length) return Promise.resolve()

      const mutationId = `${clientId}:${++sequence}`
      const mutationFields = [
        ...new Set([
          ...fields,
          ...Object.keys(credentialChanges).map((path) => {
            const parts = credentialPathForStore(storeId, path as CredentialFieldPath)
            return parts?.[0] ?? ''
          })
        ])
      ].filter(Boolean)
      for (const field of mutationFields) {
        if (!field) continue
        latestMutation.set(field, mutationId)
        pendingFields.add(field)
      }
      store.$syncStatus.pending = pendingFields.size
      latestError = null
      store.$syncStatus.error = null

      const operation = queue.then(async () => {
        const current = transport.read(storeId)
        const baseVersions = Object.fromEntries(
          mutationFields.map((field) => [
            `${storeId}.${field}`,
            current.fieldVersions[`${storeId}.${field}`] ?? 0
          ])
        )
        const metadata: StoreWriteMetadata = {
          clientId,
          mutationId,
          baseVersions,
          ...(Object.keys(credentialChanges).length ? { credentialChanges } : {})
        }
        let response: StoreWriteResult
        try {
          response = await transport.write(storeId, wirePatch, metadata)
        } catch (error) {
          const latest = transport.read(storeId)
          for (const field of mutationFields) {
            if (latestMutation.get(field) === mutationId) {
              pendingFields.delete(field)
              latestMutation.delete(field)
              setAcceptedFields(latest, [field])
            }
          }
          store.$syncStatus.pending = pendingFields.size
          throw error
        }

        if (!response.snapshot || !response.fieldVersions || !response.credentialFields) {
          const latest = transport.read(storeId)
          for (const field of mutationFields) {
            if (latestMutation.get(field) === mutationId) {
              pendingFields.delete(field)
              latestMutation.delete(field)
              setAcceptedFields(latest, [field])
            }
          }
          store.$syncStatus.pending = pendingFields.size
          throw new Error(response.error || '配置提交响应缺少权威快照')
        }
        transport.update(storeId, {
          snapshot: response.snapshot,
          fieldVersions: response.fieldVersions,
          credentialFields: response.credentialFields
        })
        const updated = transport.read(storeId)
        for (const field of mutationFields) {
          if (latestMutation.get(field) === mutationId) {
            pendingFields.delete(field)
            latestMutation.delete(field)
            setAcceptedFields(updated, [field])
          }
        }
        store.$syncStatus.pending = pendingFields.size

        if (!response.success) {
          throw new Error(response.error || '配置保存失败')
        }
        latestError = null
        store.$syncStatus.error = null
      })

      latestOperation = operation
      queue = operation.catch((error) => {
        latestError = error instanceof Error ? error : new Error(String(error))
        store.$syncStatus.error = latestError.message
        for (const field of mutationFields) {
          if (latestMutation.get(field) === mutationId) {
            pendingFields.delete(field)
            latestMutation.delete(field)
          }
        }
        store.$syncStatus.pending = pendingFields.size
      })
      return operation
    }

    store.$commit = (patch, options) => submit(patch as Record<string, unknown>, options)
    store.$whenSynced = async () => {
      await latestOperation
      if (latestError) throw latestError
    }

    store.$patch = ((patch: unknown) => {
      if (typeof patch === 'function') {
        const before = clone(store.$state) as Record<string, unknown>
        originalPatch(patch as (state: typeof store.$state) => void)
        const changed = Object.fromEntries(
          Object.keys(store.$state)
            .filter((key) => !same(before[key], store.$state[key]))
            .map((key) => [key, clone(store.$state[key])])
        )
        if (Object.keys(changed).length)
          void submit(changed).catch((error) => {
            console.error('[配置同步] 保存失败:', error)
          })
        return
      }
      if (!patch || typeof patch !== 'object' || Array.isArray(patch)) {
        throw new Error('配置变更格式无效')
      }
      void submit(patch as Record<string, unknown>).catch((error) => {
        console.error('[配置同步] 保存失败:', error)
      })
    }) as typeof store.$patch

    listeners.push(
      onSimple('store-value-changed', (change: StoreValueChange) => {
        if (!change.key.startsWith(`${storeId}.`)) return
        const field = change.key.slice(storeId.length + 1).split('.')[0]
        if (!field) return
        const current = transport.read(storeId)
        if (
          change.clientId === clientId ||
          change.source.id === clientId ||
          latestMutation.get(field) === change.mutationId
        ) {
          return
        }
        if (!pendingFields.has(field) && current.snapshot.value) {
          setAcceptedFields(current, [field])
        }
      })
    )

    const dispose = store.$dispose.bind(store)
    store.$dispose = () => {
      listeners.forEach((unsubscribe) => unsubscribe())
      dispose()
    }
  }
}
