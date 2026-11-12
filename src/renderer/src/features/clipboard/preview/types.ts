import type { ClipboardItem } from '@renderer/api/types'
import type { ClipboardColor } from '@renderer/utils/clipboard-color'

export interface FilePreview {
  path: string
  name: string
  directory: string
  extension: string
}

export interface TablePreview {
  rows: string[][]
  columns: number
}

export interface ClipboardPreview {
  kind:
    | 'text'
    | 'code'
    | 'table'
    | 'rich'
    | 'image'
    | 'files'
    | 'url'
    | 'email'
    | 'phone'
    | 'color'
    | 'empty'
  label: string
  theme: string
  text: string
  displayText: string
  html: string
  notice: string
  files: FilePreview[]
  table: TablePreview | null
  url: { href: string; host: string; detail: string } | null
  color: string | null
  colorFormats: ClipboardColor | null
  image?: Uint8Array
  formats: string[]
  summary: string
}

export interface PreviewContext {
  item: ClipboardItem
  preview: ClipboardPreview
  text: string
  trimmed: string
}

export type PreviewProvider = (context: PreviewContext) => string | null

export interface PreviewProps {
  preview: ClipboardPreview
  expanded?: boolean
  plain?: boolean
}

export interface PreviewEvents {
  imageLoad: [event: Event]
}
