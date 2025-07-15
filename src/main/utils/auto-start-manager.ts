import { app } from 'electron'
import AutoLaunch from 'auto-launch'
import logger from './logger'

/**
 * 跨平台开机自启管理器
 * Windows/macOS: 使用 Electron 原生 API
 * Linux: 使用 auto-launch 库
 */
class AutoStartManager {
  private autoLauncher: AutoLaunch | null = null
  private readonly appName: string = 'Super Paste'

  private readonly isUnix: boolean =
    process.platform === 'linux' || process.platform === 'freebsd' || process.platform === 'openbsd' || process.platform === 'darwin'

  constructor() {
    // 仅在 Linux 平台初始化 auto-launch
    if (this.isUnix) {
      this.autoLauncher = new AutoLaunch({
        mac: {
          useLaunchAgent: true
        },
        name: this.appName,
        path: process.execPath,
        isHidden: true // Linux 下隐藏启动
        
      })
    }
  }

  /**
   * 检查当前开机自启状态
   * @returns Promise<boolean> 是否已启用开机自启
   */
  async isEnabled(): Promise<boolean> {
    try {
      if (this.isUnix) {
        if (!this.autoLauncher) {
          throw new Error('Auto launcher not initialized for Linux')
        }
        return await this.autoLauncher.isEnabled()
      } else {
        // Windows 和 macOS 使用 Electron 原生 API
        const loginItemSettings = app.getLoginItemSettings()
        return loginItemSettings.openAtLogin
      }
    } catch (error) {
      logger.error('[自启管理器] 检查自启状态失败:', error)
      return false
    }
  }

  /**
   * 设置开机自启状态
   * @param enabled 是否启用开机自启
   * @returns Promise<boolean> 设置是否成功
   */
  async setEnabled(enabled: boolean): Promise<boolean> {
    try {
      if (this.isUnix) {
        if (!this.autoLauncher) {
          throw new Error('Auto launcher not initialized for Linux')
        }

        if (enabled) {
          await this.autoLauncher.enable()
        } else {
          await this.autoLauncher.disable()
        }
      } else {
        // Windows 和 macOS 使用 Electron 原生 API
        if (enabled) {
          app.setLoginItemSettings({
            openAtLogin: enabled,
            openAsHidden: true, // 自动启动时隐藏窗口
            name: this.appName,
            path: process.execPath
          })
        }else {
          app.setLoginItemSettings({
            openAtLogin: false,
          })
        }

      }

      logger.info(`[自启管理器] 自动启动设置已更新: ${enabled} (平台: ${process.platform})`)
      return true
    } catch (error) {
      logger.error('[自启管理器] 设置自启状态失败:', error)
      return false
    }
  }

  /**
   * 切换开机自启状态
   * @returns Promise<boolean> 切换后的状态
   */
  async toggle(): Promise<boolean> {
    try {
      const currentState = await this.isEnabled()
      const newState = !currentState
      const success = await this.setEnabled(newState)
      
      if (success) {
        return newState
      } else {
        return currentState
      }
    } catch (error) {
      logger.error('[自启管理器] 切换自启状态失败:', error)
      return false
    }
  }

  /**
   * 获取当前平台信息
   * @returns 平台相关信息
   */
  getPlatformInfo(): { platform: string; method: string } {
    return {
      platform: process.platform,
      method: this.isUnix ? 'auto-launch' : 'electron-native'
    }
  }
}

// 导出单例实例
export default new AutoStartManager() 