import { ref, watch, onMounted, onUnmounted, type Ref } from 'vue'

const callbacks = new Map<Element, (visible: boolean) => void>()
const retained = new Map<Element, { inside: boolean; release: () => void }>()
let observer: IntersectionObserver | undefined

// 保留近期离屏预览以便反向滚动；限制离屏图片驻留，窗口短暂隐藏不触发重建。
const MAX_RETAINED = 24
const RELEASE_DELAY = 15000

export function useDeferredPreview(
  target: Ref<HTMLElement | undefined>,
  active: Ref<boolean>
): Ref<boolean> {
  const visible = ref(false)
  let element: HTMLElement | undefined
  let inside = false
  let timer: ReturnType<typeof setTimeout> | undefined
  const release = (): void => {
    clearTimeout(timer)
    timer = undefined
    visible.value = false
    if (element) retained.delete(element)
  }
  const update = (): void => {
    clearTimeout(timer)
    timer = undefined
    if (!element || !active.value) {
      release()
      return
    }
    if (inside) {
      visible.value = true
      retained.delete(element)
      retained.set(element, { inside: true, release })
      for (const [candidate, state] of retained) {
        if (retained.size <= MAX_RETAINED) break
        if (candidate !== element && !state.inside) state.release()
      }
    } else if (visible.value) {
      const state = retained.get(element)
      if (state) state.inside = false
      timer = setTimeout(release, RELEASE_DELAY)
    }
  }
  watch(active, update)
  onMounted(() => {
    element = target.value
    if (!element) return
    observer ??= new IntersectionObserver(
      (entries) => {
        for (const entry of entries) callbacks.get(entry.target)?.(entry.isIntersecting)
      },
      { rootMargin: '400px' }
    )
    callbacks.set(element, (intersecting) => {
      inside = intersecting
      update()
    })
    observer.observe(element)
  })
  onUnmounted(() => {
    release()
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
