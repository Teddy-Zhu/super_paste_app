import { createRequire } from 'node:module'
import { basename } from 'node:path'
import type { IActiveWindow } from '@paymoapp/active-window'
import type * as KWin from '@super-paste/active-window-kwin'
import { isWayland } from '../system/session'
import logger from '../utils/logger'
import { getLinuxAppIcon } from './linux-app-icon'

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
  private diagnosticSnapshot: string | undefined

  constructor(private readonly backend: typeof KWin) {}

  async start(onChange: WindowListener, onError: ErrorListener): Promise<void> {
    const support = this.backend.getSupportStatus()
    if (!support.supported) throw new Error(support.reason)
    this.active = true
    const publish = async (window: KWin.WindowInfo | null): Promise<void> => {
      if (!this.active) return
      const diagnosticSnapshot = JSON.stringify(
        window ? [window.id, window.application, window.pid, window.path] : null
      )
      if (diagnosticSnapshot !== this.diagnosticSnapshot) {
        this.diagnosticSnapshot = diagnosticSnapshot
        logger.info('[ActiveWindowService] KWin snapshot', {
          hasWindow: Boolean(window),
          windowId: window?.id,
          application: window?.application,
          pid: window?.pid,
          hasTitle: Boolean(window?.title),
          hasPath: Boolean(window?.path),
          executable: window?.path ? basename(window.path) : undefined
        })
      }
      const version = ++this.snapshotVersion
      const snapshot = window
        ? {
            title: window.title,
            application: window.application,
            path: window.path || '',
            pid: window.pid,
            icon: await getLinuxAppIcon(window.application, window.path || '')
          }
        : null
      if (this.active && version === this.snapshotVersion) onChange(snapshot)
    }
    const queryStarted = performance.now()
    const initialWindow = await this.backend.getActiveWindow()
    logger.info('[ActiveWindowService] KWin initial query', {
      durationMs: Math.round(performance.now() - queryStarted),
      hasWindow: Boolean(initialWindow)
    })
    await publish(initialWindow)
    if (!this.active) return
    this.unsubscribe = this.backend.subscribe(
      (window) => void publish(window).catch(onError),
      (error) => {
        ++this.snapshotVersion
        if (this.active) onError(error)
      }
    )
  }

  stop(): void {
    this.active = false
    ++this.snapshotVersion
    this.unsubscribe?.()
    this.unsubscribe = null
    this.diagnosticSnapshot = undefined
  }
}

export function createActiveWindowProvider(): ActiveWindowProvider {
  logger.info('[ActiveWindowService] Backend selected', {
    backend: isWayland ? 'kwin' : 'native',
    platform: process.platform,
    arch: process.arch,
    session: process.env.XDG_SESSION_TYPE,
    desktop: process.env.XDG_CURRENT_DESKTOP,
    kdeVersion: process.env.KDE_SESSION_VERSION,
    hasWaylandDisplay: Boolean(process.env.WAYLAND_DISPLAY),
    hasSessionBus: Boolean(process.env.DBUS_SESSION_BUS_ADDRESS)
  })
  if (isWayland) {
    return new KWinActiveWindowProvider(requireNative('@super-paste/active-window-kwin'))
  }
  return new NativeActiveWindowProvider(requireNative('@paymoapp/active-window').default)
}
