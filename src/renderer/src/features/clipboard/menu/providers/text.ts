import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import IconTextBox from '~icons/mdi/text-box'
import type { ClipboardMenuProvider } from '../types'

export const textMenu: ClipboardMenuProvider = {
  matches: ({ item }) => ['text', 'html', 'url'].includes(item.primaryType),
  createItems: ({ item, callbacks }) => [
    {
      label: PREVIEW_ACTIONS.generateQRCode,
      icon: IconTextBox,
      action: () => callbacks.generateQRCode(item.id)
    }
  ]
}
