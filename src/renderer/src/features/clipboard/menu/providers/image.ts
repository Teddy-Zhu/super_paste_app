import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import IconTextBox from '~icons/mdi/text-box'
import type { ClipboardMenuProvider } from '../types'

export const imageMenu: ClipboardMenuProvider = {
  matches: ({ preview }) => preview.kind === 'image',
  createItems: ({ item, ocrBusy, callbacks }) => [
    ...(item.content.ocr?.trim()
      ? [
          {
            label: PREVIEW_ACTIONS.restoreOcrText,
            icon: IconTextBox,
            action: () => callbacks.restoreOcrText(item.id)
          }
        ]
      : []),
    {
      label: item.content.ocr?.trim() ? PREVIEW_ACTIONS.reparseOcr : PREVIEW_ACTIONS.parseOcr,
      icon: IconTextBox,
      action: () => callbacks.parseOcr(item.id),
      disabled: ocrBusy
    },
    {
      label: PREVIEW_ACTIONS.parseQRCode,
      icon: IconTextBox,
      action: () => callbacks.parseQRCode(item.id)
    }
  ]
}
