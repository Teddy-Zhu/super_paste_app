import { createHash } from 'node:crypto'
import { closeSync, openSync, readSync } from 'node:fs'

// Archives and shared libraries can be much larger than a manifest or notice.
// Keep hash working memory independent of their size in both delivery paths.
export function sha256File(file) {
  const descriptor = openSync(file, 'r')
  try {
    const hash = createHash('sha256')
    const buffer = Buffer.allocUnsafe(64 * 1024)
    for (;;) {
      const length = readSync(descriptor, buffer, 0, buffer.length, null)
      if (length === 0) break
      hash.update(buffer.subarray(0, length))
    }
    return hash.digest('hex')
  } finally {
    closeSync(descriptor)
  }
}
