import type { PreviewProvider } from '../types'
import { PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'

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

export const jsonPreview: PreviewProvider = ({ preview, trimmed }) => {
  if (
    trimmed.length > PREVIEW_LIMITS.structuredText ||
    (!trimmed.startsWith('{') && !trimmed.startsWith('['))
  )
    return null
  try {
    const json: unknown = JSON.parse(trimmed)
    if (json !== null && typeof json === 'object') {
      preview.kind = 'code'
      preview.displayText = formatJson(trimmed)
      return 'json'
    }
  } catch {
    // 非 JSON 内容继续交给后续解析器。
  }
  return null
}

export const codePreview: PreviewProvider = ({ preview, text }) => {
  if (
    !/^(?:\s*(?:const|let|var|function|class|import|export|def|public|SELECT|INSERT|UPDATE)\b|\s*```)/m.test(
      text
    )
  )
    return null
  preview.kind = 'code'
  return 'code'
}
