import { powerMonitor } from 'electron'
import clipboardManager from '../clipboard/clipboard-manager'
import { SYSTEM_RESUME_DELAY } from '../config/constants'
import logger from '../utils/logger'

/**
 * 电源监控管理器
 */
export class PowerMonitorManager {
  private isInitialized = false

  /**
   * 设置系统电源监控
   */
  setup(): void {
    if (this.isInitialized) {
      return
    }

    try {
      logger.debug('[主进程] 开始设置电源监控')

      // 监听系统挂起事件
      powerMonitor.on('suspend', this.handleSystemSuspend.bind(this))

      // 监听系统恢复事件
      powerMonitor.on('resume', this.handleSystemResume.bind(this))

      // 监听系统锁定事件（可选）
      powerMonitor.on('lock-screen', this.handleLockScreen.bind(this))

      // 监听系统解锁事件（可选）
      powerMonitor.on('unlock-screen', this.handleUnlockScreen.bind(this))

      this.isInitialized = true
      logger.debug('[主进程] 电源监控设置完成')
    } catch (error) {
      logger.error('[主进程] 设置电源监控失败:', error)
    }
  }

  /**
   * 处理系统挂起事件
   */
  private handleSystemSuspend(): void {
    logger.debug('[主进程] 系统进入休眠状态')

    // 暂停剪贴板监听以避免在休眠期间的错误
    clipboardManager.stopListening().catch(error => {
      logger.error('[主进程] 停止剪贴板监听失败:', error)
    })
  }

  /**
   * 处理系统恢复事件
   */
  private handleSystemResume(): void {
    logger.debug('[主进程] 系统从休眠中恢复')

    // 延迟处理系统恢复，给系统一些时间完全恢复
    setTimeout(async () => {
      try {
        await clipboardManager.handleSystemResume()
        logger.debug('[主进程] 系统唤醒恢复处理完成')
      } catch (error) {
        logger.error('[主进程] 系统唤醒恢复处理失败:', error)
      }
    }, SYSTEM_RESUME_DELAY)
  }

  /**
   * 处理系统锁定事件
   */
  private handleLockScreen(): void {
    logger.debug('[主进程] 系统屏幕已锁定')
  }

  /**
   * 处理系统解锁事件
   */
  private handleUnlockScreen(): void {
    logger.debug('[主进程] 系统屏幕已解锁')
  }

  /**
   * 清理资源
   */
  cleanup(): void {
    // powerMonitor 会随着应用退出自动清理，这里主要重置状态
    this.isInitialized = false
    logger.debug('[主进程] 电源监控已清理')
  }
}

// 导出单例实例
export const powerMonitorManager = new PowerMonitorManager()
