import { EventEmitter } from 'node:events'
import {
  createActiveWindowProvider,
  type ActiveWindowInfo,
  type ActiveWindowProvider
} from './active-window-provider'
import logger from '../utils/logger'

export type { ActiveWindowInfo } from './active-window-provider'

class ActiveWindowService extends EventEmitter {
  private provider: ActiveWindowProvider | null = null
  private initialization: Promise<void> | null = null
  private initialized = false
  private generation = 0
  private currentWindow: ActiveWindowInfo | null = null
  private status: { available: boolean; error?: string } = { available: false }

  private isWindowChanged(window: ActiveWindowInfo | null): boolean {
    const previous = this.currentWindow
    if (!previous || !window) return previous !== window
    return (
      previous.title !== window.title ||
      previous.application !== window.application ||
      previous.path !== window.path ||
      previous.pid !== window.pid
    )
  }

  public initialize(): Promise<void> {
    if (this.initialization) return this.initialization
    if (this.initialized && this.status.available) return Promise.resolve()
    this.cleanup()
    const generation = this.generation
    this.initialization = this.start(generation).finally(() => {
      if (generation === this.generation) this.initialization = null
    })
    return this.initialization
  }

  private async start(generation: number): Promise<void> {
    let provider: ActiveWindowProvider | undefined
    try {
      provider = createActiveWindowProvider()
      this.provider = provider
      await provider.start(
        (window) => {
          if (generation !== this.generation) return
          if (this.initialized && this.status.error) {
            logger.info('[ActiveWindowService] Connection recovered')
          }
          this.status = { available: true }
          if (!this.isWindowChanged(window)) return
          this.currentWindow = window
          this.emit('active-window-change', window)
        },
        (error) => {
          if (generation === this.generation) this.handleError(error)
        }
      )
      if (generation !== this.generation) return
      this.initialized = true
      this.status = { available: true }
      logger.info('[ActiveWindowService] Initialized')
    } catch (error) {
      if (generation !== this.generation) return
      this.stopProvider()
      this.handleError(error)
    }
  }

  private handleError(error: unknown): void {
    const message = error instanceof Error ? error.message : String(error)
    if (message !== this.status.error) logger.warn('[ActiveWindowService]', error)
    this.status = { available: false, error: message }
    if (this.currentWindow) {
      this.currentWindow = null
      this.emit('active-window-change', null)
    }
  }

  public getStatus(): { available: boolean; error?: string } {
    return { ...this.status }
  }

  public getCurrentWindow(): ActiveWindowInfo | null {
    return this.currentWindow
  }

  private stopProvider(): void {
    try {
      this.provider?.stop()
    } catch (error) {
      logger.error('[ActiveWindowService] Cleanup failed:', error)
    }
    this.provider = null
  }

  public cleanup(): void {
    ++this.generation
    this.stopProvider()
    this.initialization = null
    this.initialized = false
    this.status = { available: false }
    if (this.currentWindow) {
      this.currentWindow = null
      this.emit('active-window-change', null)
    }
  }
}

export const activeWindowService = new ActiveWindowService()
