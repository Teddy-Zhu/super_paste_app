import { onBeforeUnmount, ref, watch, type CSSProperties, type Ref } from 'vue'

type PopoverWidth = 'anchor' | number

export function useFloatingPopover(
  open: Ref<boolean>,
  anchor: Ref<HTMLElement | null>,
  panel: Ref<HTMLElement | null>,
  width: PopoverWidth = 'anchor',
  closeOnExternalScroll?: () => void
): { style: Ref<CSSProperties> } {
  const style = ref<CSSProperties>({ position: 'fixed', visibility: 'hidden' })
  let resizeObserver: ResizeObserver | undefined

  const close = (): void => {
    closeOnExternalScroll?.()
    open.value = false
  }

  const update = (): void => {
    const anchorElement = anchor.value
    const panelElement = panel.value
    if (!open.value || !anchorElement || !panelElement) return

    const rect = anchorElement.getBoundingClientRect()
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight
    const margin = 8
    const gap = 4
    if (
      rect.bottom <= 0 ||
      rect.top >= viewportHeight ||
      rect.right <= 0 ||
      rect.left >= viewportWidth
    ) {
      close()
      return
    }
    // Measure content as well as the constrained box, so flipping does not oscillate after clipping.
    const panelHeight = Math.max(
      panelElement.getBoundingClientRect().height,
      panelElement.scrollHeight + panelElement.offsetHeight - panelElement.clientHeight
    )
    const below = Math.max(0, viewportHeight - rect.bottom - gap - margin)
    const above = Math.max(0, rect.top - gap - margin)
    const openAbove = below < Math.min(panelHeight, 180) && above > below
    const availableHeight = openAbove ? above : below
    if (availableHeight <= 0) {
      close()
      return
    }
    const desiredWidth = width === 'anchor' ? rect.width : width
    const panelWidth = Math.max(0, Math.min(desiredWidth, viewportWidth - margin * 2))
    const left = Math.max(margin, Math.min(rect.left, viewportWidth - panelWidth - margin))
    const height = Math.min(panelHeight, Math.max(0, availableHeight))
    const top = openAbove ? rect.top - gap - height : rect.bottom + gap

    style.value = {
      position: 'fixed',
      top: `${Math.max(margin, Math.min(top, viewportHeight - margin - height))}px`,
      left: `${left}px`,
      width: `${panelWidth}px`,
      maxHeight: `${Math.max(0, availableHeight)}px`,
      visibility: 'visible'
    }
  }

  const onViewportChange = (): void => update()
  const onScroll = (event: Event): void => {
    const target = event.target
    if (target instanceof Node && panel.value?.contains(target)) return
    close()
  }
  const stopTracking = (): void => {
    window.removeEventListener('resize', onViewportChange)
    window.removeEventListener('scroll', onScroll, true)
    window.removeEventListener('blur', close)
    resizeObserver?.disconnect()
    resizeObserver = undefined
  }

  watch(
    open,
    (isOpen, _previous, onCleanup) => {
      onCleanup(stopTracking)
      if (!isOpen) return

      window.addEventListener('resize', onViewportChange)
      window.addEventListener('scroll', onScroll, true)
      window.addEventListener('blur', close)
      // A post-flush watcher already has DOM refs. Position before callers move keyboard focus.
      update()
      if (!open.value) return
      if (typeof ResizeObserver !== 'undefined') {
        resizeObserver = new ResizeObserver(update)
        if (anchor.value) resizeObserver.observe(anchor.value)
        if (panel.value) resizeObserver.observe(panel.value)
      }
    },
    { flush: 'post' }
  )

  onBeforeUnmount(stopTracking)
  return { style }
}
