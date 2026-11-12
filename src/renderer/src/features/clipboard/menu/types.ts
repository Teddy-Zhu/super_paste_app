import type { ClipboardItem, TagGroup } from '@renderer/api/types'
import type { ContextMenuItem } from '@renderer/composables/useContextMenu'
import type { ClipboardPreview } from '@renderer/features/clipboard/preview'

export interface ClipboardMenuCallbacks {
  restoreAsText: (itemId: string) => void
  restoreOcrText: (itemId: string) => void
  parseOcr: (itemId: string) => void
  parseQRCode: (itemId: string) => void
  generateQRCode: (itemId: string) => void
  deleted: (itemId: string) => void
}

export interface ClipboardMenuContext {
  item: ClipboardItem
  preview: ClipboardPreview
  ocrBusy: boolean
  allTags?: TagGroup[]
  activeTagId?: string
  callbacks: ClipboardMenuCallbacks
}

export interface ClipboardMenuProvider {
  matches: (context: ClipboardMenuContext) => boolean
  createItems: (context: ClipboardMenuContext) => ContextMenuItem[]
}
