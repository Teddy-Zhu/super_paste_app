import { nativeTheme } from 'electron'
import store from '../config/store'
import { STORE_KEY } from '../config/common'
import { DEFAULT_COLOR_MODE, isColorMode, type ColorMode } from '../../shared/color-mode'

export function getColorMode(): ColorMode {
  const value = store.get<unknown>(STORE_KEY.SETTINGS_COLOR_MODE, DEFAULT_COLOR_MODE)
  return isColorMode(value) ? value : DEFAULT_COLOR_MODE
}

export function applyColorMode(): void {
  // 原生菜单和所有窗口的 prefers-color-scheme 共用 Electron 的主题来源。
  nativeTheme.themeSource = getColorMode()
}

export function getWindowBackgroundColor(): string {
  return nativeTheme.shouldUseDarkColors ? '#191c22' : '#f5f6f8'
}
