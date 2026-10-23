import { getImageMimeType } from './clipboard-preview'

interface ImageURL {
  url: string
  users: number
}

interface ImageURLLease {
  url: string
  release: () => void
}

// 列表与详情共享同一份图片的 Blob；最后一个使用者卸载时释放。
const images = new WeakMap<Uint8Array, ImageURL>()

export function acquireClipboardImageURL(data: Uint8Array): ImageURLLease {
  if (!data.byteLength) throw new Error('无效的图片数据')
  let image = images.get(data)
  if (!image) {
    const bytes = new Uint8Array(data.byteLength)
    bytes.set(data)
    image = {
      url: URL.createObjectURL(new Blob([bytes.buffer], { type: getImageMimeType(data) })),
      users: 0
    }
    images.set(data, image)
  }
  const current = image
  current.users++
  let released = false
  return {
    url: current.url,
    release: () => {
      if (released) return
      released = true
      if (--current.users === 0) {
        URL.revokeObjectURL(current.url)
        images.delete(data)
      }
    }
  }
}
