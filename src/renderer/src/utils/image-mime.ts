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
