import type { ClipboardItem } from '@renderer/api/types'
import type { FilePreview, PreviewProvider } from '../types'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'

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

export const filesPreview: PreviewProvider = ({ item, preview }) => {
  const files = preview.files
  if (!files.length || (item.primaryType !== 'files' && item.content.image?.byteLength)) return null
  preview.kind = 'files'
  preview.summary = `${files.length.toLocaleString()} 个文件`
  return 'files'
}
