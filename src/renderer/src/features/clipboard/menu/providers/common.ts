import { clipboard } from '@renderer/api/renderer-api'
import { showMessage } from '@renderer/api/dialog'
import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import IconTextBox from '~icons/mdi/text-box'
import IconDelete from '~icons/mdi/delete'
import type { ClipboardMenuProvider } from '../types'
import { separator } from '../separator'

export const commonMenu: ClipboardMenuProvider = {
  matches: () => true,
  createItems: ({ item, callbacks }) => {
    const { text, html, rtf, files } = item.content
    return [
      {
        label: PREVIEW_ACTIONS.restoreAsText,
        icon: IconTextBox,
        action: () => callbacks.restoreAsText(item.id),
        disabled: !(text?.trim() || html?.trim() || rtf?.trim() || files?.length)
      }
    ]
  }
}

export const deleteMenu: ClipboardMenuProvider = {
  matches: () => true,
  createItems: ({ item, callbacks }) => [
    separator(),
    {
      label: PREVIEW_ACTIONS.delete,
      icon: IconDelete,
      action: async () => {
        try {
          const success = await clipboard.deleteItem(item.id)
          if (success) {
            callbacks.deleted(item.id)
          } else {
            await showMessage('删除失败')
          }
        } catch (error) {
          console.error('删除剪贴板项目失败:', error)
          await showMessage('删除失败: ' + (error as Error).message)
        }
      }
    }
  ]
}
