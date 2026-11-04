import { globalShortcut } from 'electron'
import { isNativeWayland } from './platform'
import logger from '../utils/logger'

import type { ShortcutStatus } from '../../shared/system-access'

export class GlobalShortcutManager {
  private currentGlobalShortcut: string | null = null
  private status: ShortcutStatus = { accelerator: '', state: 'disabled' }
  private activationSequence = 0
  private lastActivationAt: number | null = null

  getStatus(): ShortcutStatus {
    if (
      this.currentGlobalShortcut &&
      this.status.accelerator === this.currentGlobalShortcut &&
      !globalShortcut.isRegistered(this.currentGlobalShortcut)
    ) {
      this.status = {
        accelerator: this.currentGlobalShortcut,
        state: 'failed',
        error: '快捷键注册已丢失，请重新连接'
      }
    }
    return { ...this.status }
  }

  registerGlobalShortcut(shortcutKey: string, callback: () => void, retry = false): boolean {
    const accelerator = shortcutKey.trim()
    if (!accelerator) {
      this.unregisterAll()
      return true
    }
    if (accelerator === this.currentGlobalShortcut && globalShortcut.isRegistered(accelerator)) {
      if (!retry) {
        if (this.status.accelerator !== accelerator || this.status.state === 'failed') {
          this.status = { accelerator, state: isNativeWayland ? 'portal-requested' : 'registered' }
        }
        return true
      }
      globalShortcut.unregister(accelerator)
      this.currentGlobalShortcut = null
    }
    try {
      // 新键注册成功才释放旧键，避免冲突或非法组合导致原有快捷键丢失。
      if (
        !globalShortcut.register(accelerator, () => {
          // Portal 的同步返回值只代表请求已提交，实际按键送达后才能确认可用。
          if (this.currentGlobalShortcut === accelerator) {
            this.status = { accelerator, state: 'registered' }
          }
          const now = performance.now()
          logger.info(
            `[Overlay] ${JSON.stringify({
              event: 'shortcut-received',
              accelerator,
              sequence: ++this.activationSequence,
              sincePreviousMs:
                this.lastActivationAt === null ? null : Math.round(now - this.lastActivationAt)
            })}`
          )
          this.lastActivationAt = now
          callback()
        })
      ) {
        throw new Error('快捷键被占用，或桌面未提供全局快捷键服务')
      }
      if (this.currentGlobalShortcut && this.currentGlobalShortcut !== accelerator) {
        globalShortcut.unregister(this.currentGlobalShortcut)
      }
      this.currentGlobalShortcut = accelerator
      this.status = { accelerator, state: isNativeWayland ? 'portal-requested' : 'registered' }
      logger.info('[快捷键] 注册完成:', {
        ...this.status,
        backend: isNativeWayland
          ? 'wayland-portal'
          : process.platform === 'linux'
            ? 'x11'
            : process.platform
      })
      return true
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.status = { accelerator, state: 'failed', error: message }
      logger.error(`[快捷键] 注册失败: ${accelerator}`, error)
      return false
    }
  }

  unregisterAll(): void {
    globalShortcut.unregisterAll()
    this.currentGlobalShortcut = null
    this.status = { accelerator: '', state: 'disabled' }
  }
}

export const globalShortcutManager = new GlobalShortcutManager()
