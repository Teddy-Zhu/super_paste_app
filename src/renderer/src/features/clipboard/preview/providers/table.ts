import type { TablePreview, PreviewProvider } from '../types'
import { PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'

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

export const tablePreview: PreviewProvider = ({ preview, text }) => {
  const table = parseTable(text)
  if (!table) return null
  preview.kind = 'table'
  preview.table = table
  return 'table'
}
