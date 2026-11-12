import type { PreviewProvider } from '../types'

export const emailPreview: PreviewProvider = ({ preview, trimmed }) => {
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(trimmed)) return null
  preview.kind = 'email'
  return 'email'
}

export const phonePreview: PreviewProvider = ({ item, preview, text, trimmed }) => {
  if (!text) return null
  const digits = trimmed.replace(/\D/g, '').length
  const isPhone =
    item.primaryType === 'phone' ||
    (/^(?:\+[\d ()-]+|\(\d{2,4}\)[\d -]+|0\d{2,3}[- ]\d{7,8})$/.test(trimmed) &&
      digits >= 7 &&
      digits <= 15)
  if (!isPhone) return null
  preview.kind = 'phone'
  return 'phone'
}
