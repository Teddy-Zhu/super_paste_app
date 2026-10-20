import type { ClipboardItem } from '@renderer/api/types'
import {
  PREVIEW_LABELS,
  PREVIEW_LIMITS,
  PREVIEW_TYPES
} from '@renderer/constants/clipboard-preview'
import DOMPurify from 'dompurify'
import { convertRtfToHtml } from 'rtf-to-html-converter'

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
  image?: Uint8Array
  formats: string[]
  summary: string
}

export function safeDecodeUri(value: string): string {
  try {
    return decodeURI(value)
  } catch {
    return value
  }
}

export function previewExcerpt(text: string, limit: number): string {
  let end = Math.min(text.length, limit)
  if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1])) end--
  return text.slice(0, end)
}

function normalizeFilePath(value: string): string {
  if (!/^file:\/\//i.test(value)) return value
  try {
    const url = new URL(value)
    const path = decodeURIComponent(url.pathname)
    if (url.hostname && url.hostname !== 'localhost') return `//${url.hostname}${path}`
    return path.replace(/^\/(?=[a-z]:[\\/])/i, '')
  } catch {
    return value
  }
}

export function getFilePreviews(item: ClipboardItem): FilePreview[] {
  // files 是权威来源，text 常常包含同一批路径，不能合并计数。
  const paths = item.content.files?.length
    ? item.content.files
    : item.primaryType === 'files'
      ? (item.content.text || '')
          .split(/\r\n|\r|\n/)
          .filter((line) => !/^(?:#|copy$|cut$)/.test(line))
      : []
  const unique = new Map<string, FilePreview>()
  for (const rawPath of paths) {
    if (!rawPath.trim()) continue
    const path = normalizeFilePath(rawPath)
    const normalized = path.replace(/\\/g, '/')
    const isRoot = /^(?:\/|[a-z]:\/)$/i.test(normalized)
    const withoutTrailingSlash = normalized.replace(/\/+$/, '')
    const separator = withoutTrailingSlash.lastIndexOf('/')
    const name = isRoot
      ? path
      : withoutTrailingSlash.slice(separator + 1) || PREVIEW_LABELS.unknownFile
    const extension =
      !/[\\/]$/.test(path) && name.lastIndexOf('.') > 0
        ? name.slice(name.lastIndexOf('.') + 1).toUpperCase()
        : /[\\/]$/.test(path)
          ? PREVIEW_LABELS.folder
          : PREVIEW_LABELS.file
    const key = /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized
    unique.set(key, {
      path,
      name,
      directory: isRoot ? '' : normalized.slice(0, Math.max(0, separator + 1)),
      extension
    })
  }
  return [...unique.values()]
}

export function getImageMimeType(data: Uint8Array): string {
  const startsWith = (signature: number[]): boolean =>
    signature.every((byte, index) => data[index] === byte)
  if (startsWith([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return 'image/png'
  if (startsWith([0xff, 0xd8, 0xff])) return 'image/jpeg'
  if (startsWith([0x47, 0x49, 0x46, 0x38])) return 'image/gif'
  if (
    startsWith([0x52, 0x49, 0x46, 0x46]) &&
    data[8] === 0x57 &&
    data[9] === 0x45 &&
    data[10] === 0x42 &&
    data[11] === 0x50
  )
    return 'image/webp'
  if (startsWith([0x49, 0x49, 0x2a, 0x00]) || startsWith([0x4d, 0x4d, 0x00, 0x2a]))
    return 'image/tiff'
  if (startsWith([0x42, 0x4d])) return 'image/bmp'
  if (startsWith([0x00, 0x00, 0x01, 0x00])) return 'image/x-icon'
  return ''
}

export function formatByteSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

const RICH_STYLES = [
  'color',
  'background-color',
  'font-weight',
  'font-style',
  'text-decoration',
  'text-align',
  'vertical-align',
  'white-space',
  'list-style-type'
]
const BLOCK_TAGS = new Set([
  'P',
  'DIV',
  'PRE',
  'BLOCKQUOTE',
  'LI',
  'TR',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6'
])

function extractRichText(root: Node): string {
  if (root.nodeType === Node.TEXT_NODE) return root.textContent || ''
  if (root instanceof HTMLElement && root.tagName === 'BR') return '\n'
  const text = [...root.childNodes].map(extractRichText).join('')
  if (!(root instanceof HTMLElement)) return text
  if (root.tagName === 'IMG') return root.getAttribute('alt') || ''
  if (root.tagName === 'TD' || root.tagName === 'TH') return `${text}\t`
  return BLOCK_TAGS.has(root.tagName) ? `${text}\n` : text
}

function prepareRichPreview(source: string): { html: string; text: string; hasContent: boolean } {
  const fragment = DOMPurify.sanitize(source, {
    RETURN_DOM_FRAGMENT: true,
    ALLOWED_TAGS: [
      'p',
      'div',
      'span',
      'b',
      'strong',
      'i',
      'em',
      'u',
      's',
      'del',
      'mark',
      'small',
      'sub',
      'sup',
      'br',
      'pre',
      'code',
      'blockquote',
      'ul',
      'ol',
      'li',
      'table',
      'thead',
      'tbody',
      'tfoot',
      'tr',
      'td',
      'th',
      'caption',
      'hr',
      'h1',
      'h2',
      'h3',
      'h4',
      'h5',
      'h6',
      'a',
      'img'
    ],
    ALLOWED_ATTR: ['style', 'colspan', 'rowspan', 'src', 'alt', 'title', 'dir', 'start'],
    ALLOW_DATA_ATTR: false
  })
  for (const element of fragment.querySelectorAll<HTMLElement>('*')) {
    // 保留文字排版，清除定位、尺寸和外部资源样式，避免剪贴板样式覆盖面板。
    const styles = RICH_STYLES.map((name) => [name, element.style.getPropertyValue(name)])
    element.removeAttribute('style')
    for (const [name, value] of styles) {
      if (value) element.style.setProperty(name, value)
    }
    if (
      element.tagName === 'IMG' &&
      !/^data:image\/(?:png|jpeg|gif|webp|bmp|x-icon);base64,/i.test(
        element.getAttribute('src') || ''
      )
    ) {
      element.replaceWith(
        document.createTextNode(`[${element.getAttribute('alt') || PREVIEW_LABELS.externalImage}]`)
      )
    }
    for (const name of ['colspan', 'rowspan']) {
      if (element.hasAttribute(name)) {
        element.setAttribute(
          name,
          String(Math.min(20, Math.max(1, Number(element.getAttribute(name)) || 1)))
        )
      }
    }
  }
  const text = extractRichText(fragment)
    .replace(/\t\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
  const hasContent = Boolean(text || fragment.querySelector('img, table, hr'))
  const container = document.createElement('div')
  container.append(fragment)
  return { html: container.innerHTML, text, hasContent }
}

// 只将结构一致的多行数据识别为表格；支持单元格内的引号、分隔符和换行。
function parseTable(text: string): TablePreview | null {
  if (text.length > PREVIEW_LIMITS.structuredText || (!text.includes('\n') && !text.includes('\t')))
    return null
  const delimiter = text.includes('\t') ? '\t' : ','
  if (!text.includes(delimiter)) return null
  const rows: string[][] = []
  let row: string[] = []
  let cell = ''
  let quoted = false
  let closedQuote = false
  const input = text.replace(/\r\n?/g, '\n').replace(/\n$/, '')
  for (let index = 0; index < input.length; index++) {
    const character = input[index]
    if (character === '"') {
      if (quoted && input[index + 1] === '"') {
        cell += '"'
        index++
      } else if (quoted || cell === '') {
        quoted = !quoted
        closedQuote = !quoted
      } else {
        if (delimiter === ',') return null
        cell += character
      }
    } else if (!quoted && (character === delimiter || character === '\n')) {
      row.push(cell)
      cell = ''
      closedQuote = false
      if (character === '\n') {
        rows.push(row)
        row = []
      }
    } else {
      if (closedQuote && !quoted) return null
      cell += character
    }
  }
  row.push(cell)
  rows.push(row)
  const columns = rows[0]?.length || 0
  return !quoted &&
    (rows.length > 1 || delimiter === '\t') &&
    columns > 1 &&
    rows.every((entry) => entry.length === columns)
    ? { rows, columns }
    : null
}

function formatJson(text: string): string {
  // 仅调整 JSON 标记之间的空白，保留大整数、重复键和字符串的原始字面值。
  let result = ''
  let depth = 0
  let inString = false
  let escaped = false
  for (const character of text) {
    if (inString) {
      result += character
      if (escaped) escaped = false
      else if (character === '\\') escaped = true
      else if (character === '"') inString = false
    } else if (character === '"') {
      inString = true
      result += character
    } else if (character === '{' || character === '[') {
      depth++
      // 深层 JSON 保留原文，避免缩进使预览体积成倍膨胀。
      if (depth > 40) return text
      result += `${character}\n${'  '.repeat(depth)}`
    } else if (character === '}' || character === ']') {
      depth--
      result = result.trimEnd()
      if (!result.endsWith('{') && !result.endsWith('[')) result += `\n${'  '.repeat(depth)}`
      result += character
    } else if (character === ',') {
      result += `,\n${'  '.repeat(depth)}`
    } else if (character === ':') {
      result += ': '
    } else if (!/\s/.test(character)) {
      result += character
    }
    if (result.length > PREVIEW_LIMITS.richText) return text
  }
  return result
}

function getUrl(text: string): ClipboardPreview['url'] {
  if (!/^https?:\/\/\S+$/i.test(text)) return null
  try {
    const url = new URL(text)
    return {
      href: text,
      host: url.host,
      detail: safeDecodeUri(`${url.pathname}${url.search}${url.hash}`)
    }
  } catch {
    return null
  }
}

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

  if (files.length && (item.primaryType === 'files' || !content.image?.byteLength)) {
    preview.kind = 'files'
    type = 'files'
    preview.summary = `${files.length.toLocaleString()} 个文件`
  } else if (content.image?.byteLength) {
    preview.kind = 'image'
    type = 'image'
    preview.summary = [
      getImageMimeType(content.image).replace('image/', '').toUpperCase(),
      formatByteSize(content.image.byteLength)
    ]
      .filter(Boolean)
      .join(' · ')
  } else if (content.html || content.rtf) {
    type = content.html ? 'html' : 'rtf'
    const sources = [content.html, content.rtf].filter((source): source is string =>
      Boolean(source)
    )
    for (const source of sources) {
      if (source.length > PREVIEW_LIMITS.richText) {
        preview.notice = PREVIEW_LABELS.richTooLarge
        continue
      }
      try {
        const isHtml = source === content.html
        const rich = prepareRichPreview(isHtml ? source : convertRtfToHtml(source))
        preview.html = rich.hasContent ? rich.html : ''
        preview.text = text || rich.text
        if (preview.html) {
          type = isHtml ? 'html' : 'rtf'
          preview.notice = ''
          break
        }
      } catch (error) {
        console.warn('剪贴板富文本预览失败:', error)
        preview.notice = PREVIEW_LABELS.richUnavailable
      }
    }
    preview.kind = preview.html ? 'rich' : preview.text ? 'text' : 'empty'
    preview.displayText = preview.text
  } else if (text) {
    preview.kind = 'text'
    type = 'text'
    const colorCandidate =
      /^(?:#[\da-f]{3,4}|#[\da-f]{6}|#[\da-f]{8}|(?:rgb|hsl)a?\([^\r\n]+\))$/i.test(trimmed)
    if (colorCandidate && CSS.supports('color', trimmed)) {
      preview.kind = 'color'
      preview.color = trimmed
      type = 'color'
    } else if (preview.url) {
      preview.kind = 'url'
      type = 'url'
    } else if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(trimmed)) {
      preview.kind = 'email'
      type = 'email'
    } else if (
      item.primaryType === 'phone' ||
      (/^(?:\+[\d ()-]+|\(\d{2,4}\)[\d -]+|0\d{2,3}[- ]\d{7,8})$/.test(trimmed) &&
        trimmed.replace(/\D/g, '').length >= 7 &&
        trimmed.replace(/\D/g, '').length <= 15)
    ) {
      preview.kind = 'phone'
      type = 'phone'
    } else {
      if (
        trimmed.length <= PREVIEW_LIMITS.structuredText &&
        (trimmed.startsWith('{') || trimmed.startsWith('['))
      ) {
        try {
          const json: unknown = JSON.parse(trimmed)
          if (json !== null && typeof json === 'object') {
            preview.kind = 'code'
            preview.displayText = formatJson(trimmed)
            type = 'json'
          }
        } catch {
          // 非 JSON 内容继续使用代码或普通文本预览。
        }
      }
      if (preview.kind === 'text') {
        const looksLikeCode =
          /^(?:\s*(?:const|let|var|function|class|import|export|def|public|SELECT|INSERT|UPDATE)\b|\s*```)/m.test(
            text
          )
        preview.table = looksLikeCode ? null : parseTable(text)
        if (looksLikeCode) {
          preview.kind = 'code'
          type = 'code'
        } else if (preview.table) {
          preview.kind = 'table'
          type = 'table'
        } else if (item.primaryType === 'code') {
          preview.kind = 'code'
          type = 'code'
        }
      }
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
