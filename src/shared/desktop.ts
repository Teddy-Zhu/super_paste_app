import type { ConfigReadResult } from './store-sync'

export type DesktopPlatform = 'linux' | 'win32' | 'darwin'

export interface DesktopPlatformInfo {
  platform: DesktopPlatform
  wayland: boolean
  version: string
  appName: string
}

export interface DesktopBootstrapResponse {
  config: {
    settings: ConfigReadResult
    sync: ConfigReadResult
  }
  platform: DesktopPlatformInfo
}
