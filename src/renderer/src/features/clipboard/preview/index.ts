import type { ClipboardItem } from '@renderer/api/types'
import type { ClipboardPreview, PreviewProvider } from './types'
import { PREVIEW_LABELS, PREVIEW_TYPES } from '@renderer/constants/clipboard-preview'
import { filesPreview, getFilePreviews } from './providers/files'
import { imagePreview } from './providers/image'
import { colorPreview } from './providers/color'
import { richPreview } from './providers/rich'
import { getUrl, linkPreview } from './providers/link'
import { emailPreview, phonePreview } from './providers/contact'
import { jsonPreview, codePreview } from './providers/code'
import { tablePreview } from './providers/table'
import { textPreview } from './providers/text'

export type { ClipboardPreview } from './types'
export { previewExcerpt } from './providers/text'

// 按格式优先级解析；匹配后停止，避免重复解析富文本或结构化内容。
const providers: readonly PreviewProvider[] = [
  filesPreview,
  imagePreview,
  colorPreview,
  richPreview,
  linkPreview,
  emailPreview,
  phonePreview,
  jsonPreview,
  codePreview,
  tablePreview,
  textPreview
]

export function createClipboardPreview(item: ClipboardItem): ClipboardPreview {
  const { content } = item
  const text = content.text || ''
  const trimmed = text.trim()
  const files = getFilePreviews(item)
  const preview: ClipboardPreview = {
    kind: 'empty',
    ...PREVIEW_TYPES.empty,
    text,
    displayText: text,
    html: '',
    notice: '',
    files,
    table: null,
    url: getUrl(trimmed),
    color: null,
    colorFormats: null,
    image: content.image,
    formats: [],
    summary: ''
  }
  let type = item.primaryType as string
  if (text) preview.formats.push(PREVIEW_LABELS.text)
  if (content.html) preview.formats.push('HTML')
  if (content.rtf) preview.formats.push('RTF')
  if (content.image?.byteLength) preview.formats.push(PREVIEW_LABELS.image)
  if (files.length) preview.formats.push(PREVIEW_LABELS.file)

  const context = { item, preview, text, trimmed }
  for (const resolve of providers) {
    const resolvedType = resolve(context)
    if (resolvedType !== null) {
      type = resolvedType
      break
    }
  }

  if (!preview.summary && preview.text) {
    let characters = 0
    let lines = 1
    let previous = ''
    for (const character of preview.text) {
      characters++
      if (character === '\r' || (character === '\n' && previous !== '\r')) lines++
      previous = character
    }
    preview.summary = preview.table
      ? `${preview.table.rows.length} 行 × ${preview.table.columns} 列`
      : `${characters.toLocaleString()} 字符${lines > 1 ? ` · ${lines.toLocaleString()} 行` : ''}`
  }
  Object.assign(preview, PREVIEW_TYPES[type] || PREVIEW_TYPES.text)
  return preview
}
