import colorPattern from '@shared/color-pattern.json'

const COLOR = new RegExp(colorPattern, 'i')

export interface ClipboardColor {
  original: string
  hex: string
  rgb: string
}

export function parseClipboardColor(text: string): ClipboardColor | null {
  const original = text.trim()
  // 仅识别独立的数值颜色，排除变量、继承色及夹杂颜色的正文。
  if (!COLOR.test(original) || !CSS.supports('color', original)) return null
  const context = document.createElement('canvas').getContext('2d')
  if (!context) return null
  context.fillStyle = original
  const normalized = context.fillStyle
  let channels: number[]
  let alpha = 1
  if (normalized.startsWith('#')) {
    channels = [1, 3, 5].map((start) => parseInt(normalized.slice(start, start + 2), 16))
  } else {
    const match = normalized.match(/^rgba?\(([^)]+)\)$/)
    if (!match) return null
    const values = match[1].split(',').map(Number)
    channels = values.slice(0, 3)
    alpha = values[3] ?? 1
  }
  if (channels.length !== 3 || ![...channels, alpha].every(Number.isFinite)) return null
  const byte = (value: number): string => Math.round(value).toString(16).padStart(2, '0')
  return {
    original,
    hex: `#${channels.map(byte).join('')}${alpha < 1 ? byte(alpha * 255) : ''}`.toUpperCase(),
    rgb: alpha < 1 ? `rgba(${channels.join(', ')}, ${alpha})` : `rgb(${channels.join(', ')})`
  }
}
