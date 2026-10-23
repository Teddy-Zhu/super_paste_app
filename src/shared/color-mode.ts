export const COLOR_MODE_OPTIONS = [
  { value: 'system', label: '跟随系统' },
  { value: 'light', label: '亮色模式' },
  { value: 'dark', label: '暗黑模式' }
] as const

export type ColorMode = (typeof COLOR_MODE_OPTIONS)[number]['value']

export const DEFAULT_COLOR_MODE: ColorMode = 'system'

export const COLOR_MODE_TEXT = {
  title: '颜色模式',
  description: '选择应用外观，默认跟随系统'
} as const

export function isColorMode(value: unknown): value is ColorMode {
  return COLOR_MODE_OPTIONS.some((option) => option.value === value)
}
