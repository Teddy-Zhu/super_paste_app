import type { PreviewProvider } from '../types'

export function previewExcerpt(text: string, limit: number): string {
  let end = Math.min(text.length, limit)
  if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1])) end--
  return text.slice(0, end)
}

export const textPreview: PreviewProvider = ({ item, preview, text }) => {
  if (!text) return null
  preview.kind = item.primaryType === 'code' ? 'code' : 'text'
  return preview.kind
}
