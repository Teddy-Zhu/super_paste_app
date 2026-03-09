import { screen } from 'electron'
import { WINDOW_HEIGHT, MAIN_WINDOW_CONFIG } from '../../config/constants'

// 平台相关图标资源
import iconPng from '../../../../resources/icon.png?asset'
import iconIco from '../../../../build/icon/icons/icon.ico?asset'
import iconIcns from '../../../../build/icon/icons/16x16.png?asset'
import iconLinux from '../../../../build/icon/icons/32x32.png?asset'

export type OverlayPosition = 'top' | 'bottom' | 'left' | 'right'
export const DEFAULT_OVERLAY_POSITION: OverlayPosition = 'bottom'

/**
 * 窗口配置计算工具
 */
export class WindowConfigCalculator {
  static normalizeOverlayPosition(value: unknown): OverlayPosition {
    if (value === 'top' || value === 'bottom' || value === 'left' || value === 'right') {
      return value
    }
    return DEFAULT_OVERLAY_POSITION
  }

  private static getCurrentDisplayWorkArea() {
    const cursorPoint = screen.getCursorScreenPoint()
    const currentDisplay = screen.getDisplayNearestPoint(cursorPoint)
    return currentDisplay.workArea
  }

  private static calculateVisibleBoundsByWorkArea(
    workArea: Electron.Rectangle,
    overlayPosition: OverlayPosition
  ): { x: number; y: number; width: number; height: number } {
    const { x: displayX, y: displayY, width: displayWidth, height: displayHeight } = workArea

    switch (overlayPosition) {
      case 'top':
        return {
          x: displayX,
          y: displayY,
          width: displayWidth,
          height: WINDOW_HEIGHT
        }
      case 'bottom':
        return {
          x: displayX,
          y: displayY + displayHeight - WINDOW_HEIGHT,
          width: displayWidth,
          height: WINDOW_HEIGHT
        }
      case 'left':
        return {
          x: displayX,
          y: displayY,
          width: WINDOW_HEIGHT,
          height: displayHeight
        }
      case 'right':
        return {
          x: displayX + displayWidth - WINDOW_HEIGHT,
          y: displayY,
          width: WINDOW_HEIGHT,
          height: displayHeight
        }
      default:
        return {
          x: displayX,
          y: displayY + displayHeight - WINDOW_HEIGHT,
          width: displayWidth,
          height: WINDOW_HEIGHT
        }
    }
  }

  /**
   * 获取当前显示器的窗口位置信息
   */
  static calculateWindowPosition(overlayPosition: OverlayPosition = DEFAULT_OVERLAY_POSITION): {
    x: number
    y: number
    width: number
    height: number
  } {
    return this.calculateOverlayBounds(overlayPosition).visibleBounds
  }

  static calculateHiddenWindowPosition(
    overlayPosition: OverlayPosition = DEFAULT_OVERLAY_POSITION
  ): { x: number; y: number; width: number; height: number } {
    return this.calculateOverlayBounds(overlayPosition).hiddenBounds
  }

  static calculateOverlayBounds(overlayPosition: OverlayPosition = DEFAULT_OVERLAY_POSITION): {
    visibleBounds: { x: number; y: number; width: number; height: number }
    hiddenBounds: { x: number; y: number; width: number; height: number }
  } {
    const workArea = this.getCurrentDisplayWorkArea()
    const visibleBounds = this.calculateVisibleBoundsByWorkArea(workArea, overlayPosition)
    const { x: displayX, y: displayY, width: displayWidth, height: displayHeight } = workArea

    const hiddenBounds =
      overlayPosition === 'top'
        ? { ...visibleBounds, y: displayY - visibleBounds.height }
        : overlayPosition === 'bottom'
          ? { ...visibleBounds, y: displayY + displayHeight }
          : overlayPosition === 'left'
            ? { ...visibleBounds, x: displayX - visibleBounds.width }
            : overlayPosition === 'right'
              ? { ...visibleBounds, x: displayX + displayWidth }
              : { ...visibleBounds, y: displayY + displayHeight }

    return {
      visibleBounds,
      hiddenBounds
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
