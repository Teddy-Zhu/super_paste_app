import { Tray, Menu } from 'electron'
import { IconSelector } from '../windows/window-config'
import { mainWindowManager, overlayWindowManager } from '../windows/window-managers'
import logger from '../../utils/logger'
import store from '../../config/store'
import { STORE_KEY } from '../../config/common'
import autoStartManager from '../../system/auto-start-manager'
import clipboardManager from '../../features/clipboard/manager/clipboard-manager'
import { appEventBus } from '../../utils/app-event-bus'

/**
 * 系统托盘管理器
 */
export class TrayManager {
  private tray: Tray | null = null
  private updatingOCR = false

  /**
   * 创建任务栏图标
   */
  createTray(): void {
    try {
      if (this.tray) {
        return
      }

      logger.debug('[主进程] 开始创建系统托盘')

      // 创建任务栏图标
      this.tray = new Tray(IconSelector.getTrayIcon())

      // 设置任务栏图标的工具提示
      this.tray.setToolTip('Super Paste - 剪贴板管理工具')

      this.setupContextMenu()
      this.setupEventHandlers()

      logger.debug('[主进程] 系统托盘创建完成')
    } catch (error) {
      logger.error('[主进程] 创建系统托盘时发生错误:', error)
      throw error
    }
  }

  /**
   * 设置右键菜单
   */
  private setupContextMenu(): void {
    if (!this.tray) return

    // 获取当前配置状态
    const autoStart = store.get(STORE_KEY.SETTINGS_AUTO_START, false) as boolean
    const enableLocalOCR = store.get(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, false) as boolean

    // 创建右键菜单
    const contextMenu = Menu.buildFromTemplate([
      {
        label: '设置',
        click: () => {
          mainWindowManager.showWindow()
        }
      },
      {
        label: '粘贴面板',
        click: () => {
          overlayWindowManager.toggleWindow(true, 'tray-menu')
        }
      },
      {
        type: 'separator'
      },
      {
        label: '开机自启',
        type: 'checkbox',
        checked: autoStart,
        click: async () => {
          const newValue = !autoStart
          store.set(STORE_KEY.SETTINGS_AUTO_START, newValue, {
            source: { kind: 'main', id: 'tray' }
          })

          // 实际执行自动启动设置
          const success = await autoStartManager.setEnabled(newValue)
          if (!success) {
            logger.error('[主进程] 从托盘设置自动启动失败')
          } else {
            logger.debug(`[主进程] 从托盘更新自动启动状态: ${newValue}`)
          }

          // 重新构建菜单以更新勾选状态
          this.setupContextMenu()
        }
      },
      {
        label: '自动 OCR',
        type: 'checkbox',
        checked: enableLocalOCR,
        enabled: !this.updatingOCR,
        click: async (menuItem) => {
          const newValue = menuItem.checked
          this.updatingOCR = true
          this.setupContextMenu()

          // 实际更新剪贴板管理器的 OCR 配置
          try {
            await clipboardManager.updateConfig({ enableLocalOCR: newValue })
            store.set(STORE_KEY.SETTINGS_ENABLE_LOCAL_OCR, newValue, {
              source: { kind: 'main', id: 'tray' }
            })
            logger.debug(`[主进程] 从托盘更新自动 OCR 状态: ${newValue}`)
          } catch (error) {
            logger.error('[主进程] 从托盘更新自动 OCR 配置失败:', error)
          } finally {
            this.updatingOCR = false
          }

          // 重新构建菜单以更新勾选状态
          this.setupContextMenu()
        }
      },
      {
        type: 'separator'
      },
      {
        label: '退出',
        click: () => {
          logger.info('[托盘管理器] 用户点击退出菜单')
          // 通过事件总线发送退出请求
          appEventBus.requestAppQuit()
        }
      }
    ])

    // 设置右键菜单
    this.tray.setContextMenu(contextMenu)
  }

  /**
   * 设置事件处理器
   */
  private setupEventHandlers(): void {
    if (!this.tray) return

    // 设置左键点击事件（显示快捷面板）
    this.tray.on('click', () => {
      overlayWindowManager.toggleWindow(true, 'tray-click')
    })

    // 设置双击事件（显示主窗口）
    this.tray.on('double-click', () => {
      mainWindowManager.showWindow()
    })
  }

  /**
   * 销毁托盘
   */
  destroy(): void {
    if (this.tray && !this.tray.isDestroyed()) {
      this.tray.destroy()
      this.tray = null
      logger.debug('[主进程] 系统托盘已销毁')
    }
  }

  /**
   * 检查托盘是否存在
   */
  isValid(): boolean {
    return this.tray !== null && !this.tray.isDestroyed()
  }

  /**
   * 更新托盘菜单（当配置变更时调用）
   */
  updateMenu(): void {
    if (this.isValid()) {
      this.setupContextMenu()
      logger.debug('[主进程] 托盘菜单已更新')
    }
  }
}

// 导出单例实例
export const trayManager = new TrayManager()
