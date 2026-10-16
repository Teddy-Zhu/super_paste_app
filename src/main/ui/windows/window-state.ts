import { BrowserWindow } from 'electron'

/**
 * 窗口状态枚举
 */
export enum OverlayState {
  CLOSED = 'CLOSED', // 窗口已关闭/隐藏
  OPENING = 'OPENING', // 正在打开（显示窗口 + 滑入动画）
  OPENED = 'OPENED', // 已打开（动画完成，可以交互）
  CLOSING = 'CLOSING' // 正在关闭（滑出动画 + 隐藏窗口）
}

/**
 * 窗口状态管理器
 */
class WindowStateManager {
  private currentOverlayState: OverlayState = OverlayState.CLOSED

  /**
   * 设置overlay状态
   * @param newState 新状态
   */
  setOverlayState(newState: OverlayState): void {
    this.currentOverlayState = newState
  }

  /**
   * 获取当前overlay状态
   */
  getOverlayState(): OverlayState {
    return this.currentOverlayState
  }

  /**
   * 重置状态
   */
  reset(): void {
    this.currentOverlayState = OverlayState.CLOSED
  }
}

/**
 * 窗口工具函数
 */
export class WindowUtils {
  /**
   * 安全地发送 IPC 消息到窗口
   * @param window 目标窗口
   * @param channel 消息通道
   * @param args 消息参数
   */
  static safelyEmitToWindow(
    window: BrowserWindow | null,
    channel: string,
    ...args: unknown[]
  ): void {
    if (window && !window.isDestroyed() && window.webContents) {
      window.webContents.send(channel, ...args)
    }
  }

  /**
   * 检查窗口是否有效且可见
   * @param window 要检查的窗口
   */
  static isWindowValidAndVisible(window: BrowserWindow | null): boolean {
    return window !== null && !window.isDestroyed() && window.isVisible()
  }

  /**
   * 检查窗口是否有效
   * @param window 要检查的窗口
   */
  static isWindowValid(window: BrowserWindow | null): boolean {
    return window !== null && !window.isDestroyed()
  }

  /**
   * 安全地销毁窗口
   * @param window 要销毁的窗口
   */
  static safelyDestroyWindow(window: BrowserWindow | null): void {
    if (window && !window.isDestroyed()) {
      window.destroy()
    }
  }

  /**
   * 安全地显示窗口
   * @param window 要显示的窗口
   */
  static safelyShowWindow(window: BrowserWindow | null): void {
    if (this.isWindowValid(window) && !window!.isVisible()) {
      window!.show()
    }
  }

  /**
   * 安全地隐藏窗口
   * @param window 要隐藏的窗口
   */
  static safelyHideWindow(window: BrowserWindow | null): void {
    if (this.isWindowValidAndVisible(window)) {
      window!.hide()
    }
  }

  /**
   * 安全地聚焦窗口
   * @param window 要聚焦的窗口
   */
  static safelyFocusWindow(window: BrowserWindow | null): void {
    if (this.isWindowValid(window)) {
      window!.focus()
    }
  }
}

// 导出单例实例
export const windowStateManager = new WindowStateManager()
