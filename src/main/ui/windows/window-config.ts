import { screen } from 'electron'
import { WINDOW_HEIGHT, MAIN_WINDOW_CONFIG } from '../../config/constants'

// 平台相关图标资源
import iconPng from '../../../../resources/icon.png?asset'
import iconIco from '../../../../build/icon/icons/icon.ico?asset'
import iconIcns from '../../../../build/icon/icons/16x16.png?asset'
import iconLinux from '../../../../build/icon/icons/32x32.png?asset'

/**
 * 窗口配置计算工具
 */
export class WindowConfigCalculator {
  /**
   * 获取当前显示器的窗口位置信息
   */
  static calculateWindowPosition(): { x: number; y: number; width: number; height: number } {
    const cursorPoint = screen.getCursorScreenPoint()
    const currentDisplay = screen.getDisplayNearestPoint(cursorPoint)
    const { x: displayX, y: displayY, width, height } = currentDisplay.workArea

    // 窗口宽度为屏幕宽度，位置在屏幕最底部
    const windowWidth = width
    const windowX = displayX
    const windowY = displayY + height - WINDOW_HEIGHT

    return {
      x: windowX,
      y: windowY,
      width: windowWidth,
      height: WINDOW_HEIGHT
    }
  }

  /**
   * 根据屏幕大小计算最佳的主窗口尺寸
   */
  static calculateMainWindowSize(): { width: number; height: number } {
    const primaryDisplay = screen.getPrimaryDisplay()
    const { width: screenWidth, height: screenHeight } = primaryDisplay.workArea

    // 根据屏幕尺寸计算最佳窗口大小
    let windowWidth = Math.floor(screenWidth * MAIN_WINDOW_CONFIG.WIDTH_RATIO)
    windowWidth = Math.max(
      MAIN_WINDOW_CONFIG.MIN_WIDTH,
      Math.min(MAIN_WINDOW_CONFIG.MAX_WIDTH, windowWidth)
    )

    let windowHeight = Math.floor(screenHeight * MAIN_WINDOW_CONFIG.HEIGHT_RATIO)
    windowHeight = Math.max(
      MAIN_WINDOW_CONFIG.MIN_HEIGHT,
      Math.min(MAIN_WINDOW_CONFIG.MAX_HEIGHT, windowHeight)
    )

    return {
      width: windowWidth,
      height: windowHeight
    }
  }
}

/**
 * 图标选择工具
 */
export class IconSelector {
  /**
   * 根据操作系统平台选择合适的托盘图标
   */
  static getTrayIcon(): string {
    switch (process.platform) {
      case 'win32':
        return iconIco
      case 'darwin':
        return iconIcns
      case 'linux':
        return iconLinux
      default:
        return iconPng
    }
  }

  /**
   * 根据操作系统平台选择合适的窗口图标
   */
  static getWindowIcon(): string {
    switch (process.platform) {
      case 'linux':
        return iconLinux
      default:
        return iconPng
    }
  }
}
