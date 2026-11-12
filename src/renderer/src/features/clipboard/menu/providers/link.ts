import { send } from '@renderer/api/ipc'
import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import IconOpenInNew from '~icons/mdi/open-in-new'
import type { ClipboardMenuProvider } from '../types'

export const linkMenu: ClipboardMenuProvider = {
  matches: ({ preview }) => preview.kind === 'url' && Boolean(preview.url),
  createItems: ({ preview }) => {
    const url = preview.url
    if (!url) return []
    return [
      {
        label: PREVIEW_ACTIONS.openInBrowser,
        icon: IconOpenInNew,
        action: () => send('open-external-url', url.href)
      }
    ]
  }
}
