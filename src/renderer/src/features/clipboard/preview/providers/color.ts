import type { PreviewProvider } from '../types'
import { parseClipboardColor } from '@renderer/utils/clipboard-color'

export const colorPreview: PreviewProvider = ({ preview, trimmed }) => {
  const color = parseClipboardColor(trimmed)
  if (!color) return null
  preview.kind = 'color'
  preview.color = color.original
  preview.colorFormats = color
  return 'color'
}
