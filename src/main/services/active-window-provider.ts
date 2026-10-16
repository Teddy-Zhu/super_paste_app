import { app } from 'electron'
import { createRequire } from 'node:module'
import type { IActiveWindow } from '@paymoapp/active-window'
import type * as KWin from '@super-paste/active-window-kwin'
import { isWayland } from '../system/session'
import logger from '../utils/logger'

export interface ActiveWindowInfo {
  title: string
  application: string
  path: string
  pid: number
  icon: string
}

type WindowListener = (window: ActiveWindowInfo | null) => void
type ErrorListener = (error: unknown) => void

export interface ActiveWindowProvider {
  start(onChange: WindowListener, onError: ErrorListener): Promise<void>
  stop(): void
}

// Load only the selected native backend. Wayland must not load the X11 addon
// just to import its TypeScript interface.
const requireNative = createRequire(import.meta.url)

class NativeActiveWindowProvider implements ActiveWindowProvider {
  private subscriptionId: number | null = null

  constructor(private readonly backend: IActiveWindow) {}

  async start(onChange: WindowListener): Promise<void> {
    this.backend.initialize()
    if (process.platform === 'darwin' && !this.backend.requestPermissions()) {
      logger.warn('[ActiveWindowService] macOS Screen Recording permission required')
    }
    this.subscriptionId = this.backend.subscribe(onChange)
  }

  stop(): void {
    if (this.subscriptionId !== null) {
      this.backend.unsubscribe(this.subscriptionId)
      this.subscriptionId = null
    }
  }
}

class KWinActiveWindowProvider implements ActiveWindowProvider {
  private unsubscribe: (() => void) | null = null
  private active = false
  private snapshotVersion = 0
  private readonly icons = new Map<string, Promise<string>>()

  constructor(private readonly backend: typeof KWin) {}

  async start(onChange: WindowListener, onError: ErrorListener): Promise<void> {
    const support = this.backend.getSupportStatus()
    if (!support.supported) throw new Error(support.reason)
    this.active = true
    const publish = async (window: KWin.WindowInfo | null): Promise<void> => {
      if (!this.active) return
      const version = ++this.snapshotVersion
      const snapshot = window
        ? {
            title: window.title,
            application: window.application,
            path: window.path || '',
            pid: window.pid,
            icon: window.path ? await this.getIcon(window.path) : ''
          }
        : null
      if (this.active && version === this.snapshotVersion) onChange(snapshot)
    }
    await publish(await this.backend.getActiveWindow())
    if (!this.active) return
    this.unsubscribe = this.backend.subscribe(
      (window) => void publish(window).catch(onError),
      (error) => {
        ++this.snapshotVersion
        if (this.active) onError(error)
      }
    )
  }

  private getIcon(path: string): Promise<string> {
    let icon = this.icons.get(path)
    if (!icon) {
      // Icons are optional. Cache misses as well, so an unavailable icon does
      // not trigger repeated filesystem work on every foreground update.
      icon = app.getFileIcon(path, { size: 'small' }).then(
        (image) => image.toDataURL(),
        () => ''
      )
      if (this.icons.size >= 64) this.icons.delete(this.icons.keys().next().value!)
      this.icons.set(path, icon)
    }
    return icon
  }

  stop(): void {
    this.active = false
    ++this.snapshotVersion
    this.unsubscribe?.()
    this.unsubscribe = null
    this.icons.clear()
  }
}

export function createActiveWindowProvider(): ActiveWindowProvider {
  if (isWayland) {
    return new KWinActiveWindowProvider(requireNative('@super-paste/active-window-kwin'))
  }
  return new NativeActiveWindowProvider(requireNative('@paymoapp/active-window').default)
}
