export enum SyncStatus {
  IDLE = 'idle',
  SYNCING = 'syncing',
  SUCCESS = 'success',
  ERROR = 'error'
}

export enum SyncDirection {
  UPLOAD = 'upload',
  DOWNLOAD = 'download',
  BIDIRECTIONAL = 'bidirectional'
}

export enum SyncProviderType {
  WEBDAV = 'webdav',
  FTP = 'ftp',
  SFTP = 'sftp',
  CLOUD_STORAGE = 'cloud_storage'
}

export interface WebDAVConfig {
  syncDirection: SyncDirection
  serverUrl: string
  username: string
  password?: string
  remotePath: string
  timeout?: number
  useHttps?: boolean
}

export interface ScheduledSyncConfig {
  enabled: boolean
  syncInterval: number
  providerType: SyncProviderType
  providerConfig: WebDAVConfig
  userDataPath: string
}

export interface ScheduledSyncStatus {
  isRunning: boolean
  lastSyncTime: number
  nextSyncTime: number
  syncStatus: SyncStatus
  config: ScheduledSyncConfig | null
}

export interface SyncResult {
  success: boolean
  message: string
  uploadedFiles?: string[]
  downloadedFiles?: string[]
  errors?: string[]
}
