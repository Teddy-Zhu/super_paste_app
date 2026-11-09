import type { DesktopBootstrapResponse, DesktopPlatformInfo } from '@shared/desktop'
import { initializeIpcEvents, installDesktopConfig, invoke } from './ipc'

let platformInfo: DesktopPlatformInfo | undefined

export async function initializeDesktop(): Promise<DesktopBootstrapResponse> {
  await initializeIpcEvents()
  const bootstrap = await invoke<DesktopBootstrapResponse>('desktop_bootstrap')
  installDesktopConfig(bootstrap.config)
  platformInfo = bootstrap.platform
  return bootstrap
}

export function getDesktopPlatform(): DesktopPlatformInfo {
  if (!platformInfo) throw new Error('桌面平台信息尚未初始化')
  return platformInfo
}
