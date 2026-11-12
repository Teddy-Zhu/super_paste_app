import type { PreviewProvider } from '../types'
import type { ClipboardPreview } from '../types'

export function safeDecodeUri(value: string): string {
  try {
    return decodeURI(value)
  } catch {
    return value
  }
}

export function getUrl(text: string): ClipboardPreview['url'] {
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

export const linkPreview: PreviewProvider = ({ preview, text }) => {
  if (!text || !preview.url) return null
  preview.kind = 'url'
  return 'url'
}
