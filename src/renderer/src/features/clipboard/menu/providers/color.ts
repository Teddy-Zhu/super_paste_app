import { clipboard } from '@renderer/api/renderer-api'
import { showMessage } from '@renderer/api/dialog'
import IconTextBox from '~icons/mdi/text-box'
import type { ClipboardMenuProvider } from '../types'

export const colorMenu: ClipboardMenuProvider = {
  matches: ({ preview }) => preview.kind === 'color',
  createItems: ({ preview }) => {
    const color = preview.colorFormats
    if (!color) return []
    return (['hex', 'rgb'] as const).map((format) => ({
      label: `复制为 ${format.toUpperCase()}`,
      icon: IconTextBox,
      action: async () => {
        try {
          await clipboard.copyColor(color[format])
        } catch (error) {
          await showMessage('复制颜色失败: ' + (error as Error).message)
        }
      }
    }))
  }
}
