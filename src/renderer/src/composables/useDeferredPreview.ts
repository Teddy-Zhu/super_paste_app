import { ref, onMounted, onUnmounted, type Ref } from 'vue'

// 历史卡片共用观察器；只在可见区域附近解析富文本、解码图片。
const callbacks = new Map<Element, (visible: boolean) => void>()
let observer: IntersectionObserver | undefined

export function useDeferredPreview(target: Ref<HTMLElement | undefined>): Ref<boolean> {
  const visible = ref(false)
  let element: HTMLElement | undefined
  onMounted(() => {
    element = target.value
    if (!element) return
    observer ??= new IntersectionObserver(
      (entries) => {
        for (const entry of entries) callbacks.get(entry.target)?.(entry.isIntersecting)
      },
      { rootMargin: '400px' }
    )
    callbacks.set(element, (value) => {
      visible.value = value
    })
    observer.observe(element)
  })
  onUnmounted(() => {
    if (element) {
      observer?.unobserve(element)
      callbacks.delete(element)
    }
    if (!callbacks.size) {
      observer?.disconnect()
      observer = undefined
    }
  })
  return visible
}
