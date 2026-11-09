import { isColorMode } from '@shared/color-mode'

export function applyColorScheme(mode: unknown, systemPrefersDark: boolean): void {
  const colorMode = isColorMode(mode) ? mode : 'system'
  const isDark = colorMode === 'dark' || (colorMode === 'system' && systemPrefersDark)
  document.documentElement.dataset.colorScheme = isDark ? 'dark' : 'light'
}
