import { globalShortcut } from 'electron'
import { isWayland } from './platform'
import logger from '../utils/logger'

import type { ShortcutStatus } from '../../shared/system-access'

export class GlobalShortcutManager {
  private currentGlobalShortcut: string | null = null
  private status: ShortcutStatus = { accelerator: '', state: 'disabled' }
  private activationSequence = 0
  private lastActivationAt: number | null = null

  getStatus(): ShortcutStatus {
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
        this.status = { accelerator, state: isWayland ? 'portal-requested' : 'registered' }
        return true
      }
      globalShortcut.unregister(accelerator)
      this.currentGlobalShortcut = null
    }
    try {
      // 新键注册成功才释放旧键，避免冲突或非法组合导致原有快捷键丢失。
      if (
        !globalShortcut.register(accelerator, () => {
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
      if (this.currentGlobalShortcut) globalShortcut.unregister(this.currentGlobalShortcut)
      this.currentGlobalShortcut = accelerator
      // Portal 授权是异步的，Electron 37 的返回值不能证明用户已授权。
      this.status = { accelerator, state: isWayland ? 'portal-requested' : 'registered' }
      logger.info('[快捷键] 注册完成:', this.status)
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
