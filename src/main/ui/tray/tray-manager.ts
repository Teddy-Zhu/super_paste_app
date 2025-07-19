import { Tray, Menu, app } from 'electron'
import { IconSelector } from '../windows/window-config'
import { mainWindowManager, overlayWindowManager } from '../windows/window-managers'
import logger from '../../utils/logger'

/**
 * 系统托盘管理器
 */
export class TrayManager {
  private tray: Tray | null = null

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

    // 创建右键菜单
    const contextMenu = Menu.buildFromTemplate([
      {
        label: '设置',
        click: () => {
          mainWindowManager.showWindow()
        }
      },
      {
        label: '快捷面板',
        click: () => {
          overlayWindowManager.toggleWindow(true)
        }
      },
      {
        type: 'separator'
      },
      {
        label: '退出',
        click: () => {
          app.quit()
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
      overlayWindowManager.toggleWindow(true)
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
}

// 导出单例实例
export const trayManager = new TrayManager()
