import { app } from 'electron'

import { isWayland } from './session'
export { isWayland } from './session'

export function configurePlatform(): void {
  if (process.platform !== 'linux') return

  if (isWayland) {
    // 必须在 ready 之前开启；保留调用方传入的其他 Chromium 特性。
    const features = new Set(
      app.commandLine.getSwitchValue('enable-features').split(',').filter(Boolean)
    )
    features.add('GlobalShortcutsPortal')
    app.commandLine.appendSwitch('enable-features', [...features].join(','))
  }
}
