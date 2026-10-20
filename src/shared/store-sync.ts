export interface StoreChangeSource {
  kind: 'main' | 'renderer'
  id: string
  windowId?: number
}

export interface StoreMutationContext {
  source: StoreChangeSource
  mutationId?: string
}

export interface StoreWriteMetadata {
  clientId: string
  mutationId: string
}

export interface StoreValueChange extends StoreMutationContext {
  key: string
  value: unknown
  version: number
  timestamp: number
}

export interface StoreSnapshot {
  value: Record<string, unknown> | undefined
  version: number
}

export interface StoreWriteResult {
  success: boolean
  snapshot?: StoreSnapshot
  error?: string
}
