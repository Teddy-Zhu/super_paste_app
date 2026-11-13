import type { PreviewProvider } from '../types'

export function formatByteSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export const imagePreview: PreviewProvider = ({ item, preview }) => {
  const image = item.content.image
  if (typeof image !== 'string' || !image) return null
  preview.kind = 'image'
  preview.summary = '图片'
  return 'image'
}
