import { app } from 'electron'

import { isWayland, useXWayland } from './session'
export { isWayland } from './session'

export function configurePlatform(): boolean {
  if (process.platform !== 'linux') return true

  // 直接启动安装包时也要在进程启动参数中选定后端；JS 中 appendSwitch 已太迟。
  if (useXWayland && app.commandLine.getSwitchValue('ozone-platform') !== 'x11') {
    app.relaunch({ args: [...process.argv.slice(1), '--ozone-platform=x11'] })
    app.exit(0)
    return false
  }

  if (isWayland) {
    // 必须在 ready 之前开启；保留调用方传入的其他 Chromium 特性。
    const features = new Set(
      app.commandLine.getSwitchValue('enable-features').split(',').filter(Boolean)
    )
    features.add('GlobalShortcutsPortal')
    app.commandLine.appendSwitch('enable-features', [...features].join(','))
  }
  return true
}
