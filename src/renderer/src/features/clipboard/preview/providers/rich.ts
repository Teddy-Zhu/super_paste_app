import type { PreviewProvider } from '../types'
import { PREVIEW_LABELS, PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import DOMPurify from 'dompurify'
import { convertRtfToHtml } from 'rtf-to-html-converter'

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

export const richPreview: PreviewProvider = ({ item, preview, text }) => {
  const { content } = item
  if (!content.html && !content.rtf) return null
  let type: string
  type = content.html ? 'html' : 'rtf'
  const sources = [content.html, content.rtf].filter((source): source is string => Boolean(source))
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
  return type
}
