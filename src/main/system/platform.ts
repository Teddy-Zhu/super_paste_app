import { app } from 'electron'

import { isWayland } from './session'
export { isWayland } from './session'

// Electron 默认按会话选择 Ozone 后端；显式启动参数仍可覆盖默认选择。
const ozonePlatform = app.commandLine.getSwitchValue('ozone-platform')
export const isNativeWayland =
  process.platform === 'linux' &&
  (ozonePlatform === 'wayland' || ((!ozonePlatform || ozonePlatform === 'auto') && isWayland))

export function configurePlatform(): void {
  if (!isNativeWayland) return

  // 必须在 ready 之前开启；保留调用方传入的其他 Chromium 特性。
  const features = new Set(
    app.commandLine.getSwitchValue('enable-features').split(',').filter(Boolean)
  )
  features.add('GlobalShortcutsPortal')
  features.add('GlobalShortcutsPortalPreferredTrigger')
  app.commandLine.appendSwitch('enable-features', [...features].join(','))
}
