import { shallowRef, watch, type CSSProperties, type Ref } from 'vue'
import { useResizeObserver } from '@vueuse/core'

export function useTabIndicator(
  container: Ref<HTMLElement | undefined>,
  tabs: Readonly<Ref<readonly unknown[]>>
): Readonly<Ref<CSSProperties>> {
  const elements = shallowRef<HTMLElement[]>([])
  const style = shallowRef<CSSProperties>({ opacity: 0 })
  let activeElement: HTMLElement | null = null

  const update = (): void => {
    const active = container.value?.querySelector<HTMLElement>('.tab.active')
    if (!active) {
      activeElement = null
      style.value = { opacity: 0 }
      return
    }
    // 搜索期间标签栏会隐藏，保留有效坐标，显示后由尺寸观察器重新定位。
    if (!active.offsetWidth || !active.offsetHeight) return
    const geometry = {
      width: `${active.offsetWidth}px`,
      height: `${active.offsetHeight}px`,
      transform: `translate(${active.offsetLeft}px, ${active.offsetTop}px)`
    }
    const selectionChanged = active !== activeElement
    activeElement = active
    if (
      !selectionChanged &&
      geometry.width === style.value.width &&
      geometry.height === style.value.height &&
      geometry.transform === style.value.transform
    ) {
      return
    }
    // 尺寸同步不重播选中动画，只有实际切换标签时才移动背景。
    style.value = {
      ...geometry,
      opacity: 1,
      transition: selectionChanged && style.value.width ? undefined : 'none'
    }
  }

  watch(
    [container, tabs],
    () => {
      const nextElements = container.value
        ? [container.value, ...container.value.querySelectorAll<HTMLElement>('.tab')]
        : []
      if (
        nextElements.length !== elements.value.length ||
        nextElements.some((element, index) => element !== elements.value[index])
      ) {
        elements.value = nextElements
      }
      update()
    },
    { immediate: true, flush: 'post' }
  )
  useResizeObserver(elements, update)

  return style
}
