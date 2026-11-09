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
  baseVersions: Record<string, number>
  credentialChanges?: Partial<Record<CredentialFieldPath, 'set' | 'unset'>>
}

export interface StoreValueChange extends StoreMutationContext {
  clientId?: string
  key: string
  value: unknown
  version: number
  timestamp: number
  credentialFields?: CredentialFields
}

export type CredentialFieldState = 'available' | 'configured' | 'unset' | 'unavailable' | 'error'
export type CredentialFields = Record<string, CredentialFieldState>

export type CredentialChanges = Partial<Record<CredentialFieldPath, 'set' | 'unset'>>

export const CREDENTIAL_FIELD_PATHS = [
  'settings.ocrAIConfigs.openai.apiKey',
  'settings.ocrAIConfigs.claude.apiKey',
  'sync.params.webdav.password'
] as const

export type CredentialFieldPath = (typeof CREDENTIAL_FIELD_PATHS)[number]

export interface StoreSnapshot {
  value: Record<string, unknown> | undefined
  version: number
}

export interface ConfigReadResult {
  snapshot: StoreSnapshot
  fieldVersions: Record<string, number>
  credentialFields: CredentialFields
}

export interface StoreWriteResult {
  success: boolean
  snapshot?: StoreSnapshot
  fieldVersions?: Record<string, number>
  credentialFields?: CredentialFields
  error?: string
}
