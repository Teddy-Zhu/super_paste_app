import { clipboard } from '@renderer/api/renderer-api'
import { send } from '@renderer/api/ipc'
import { showMessage } from '@renderer/api/dialog'
import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import { createClipboardPreview } from '@renderer/features/clipboard/preview'
import IconOpenInNew from '~icons/mdi/open-in-new'
import type { ClipboardMenuProvider } from '../types'

export const linkMenu: ClipboardMenuProvider = {
  matches: ({ preview }) => preview.kind === 'url' && Boolean(preview.url),
  createItems: ({ item, preview }) => {
    const url = preview.url
    if (!url) return []
    const textCharacters = [...(item.content.text || '')].length
    const needsFullItem =
      Boolean(item.previewTruncated) ||
      (typeof item.textLength === 'number' && item.textLength > textCharacters)
    return [
      {
        label: PREVIEW_ACTIONS.openInBrowser,
        icon: IconOpenInNew,
        action: async () => {
          try {
            const fullUrl = needsFullItem
              ? createClipboardPreview(await clipboard.getItem(item.id)).url?.href
              : url.href
            if (!fullUrl) throw new Error('链接内容无效')
            send('open-external-url', fullUrl)
          } catch (error) {
            await showMessage(
              `打开链接失败: ${error instanceof Error ? error.message : String(error)}`
            )
          }
        }
      }
    ]
  }
}
