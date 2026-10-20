import { reactive, type Component } from 'vue'

export interface ContextMenuItem {
  label: string
  action: () => void
  icon?: string | Component
  iconColor?: string
  disabled?: boolean
  submenu?: ContextMenuItem[]
}

export interface ContextMenuState {
  visible: boolean
  x: number
  y: number
  items: ContextMenuItem[]
  sourceId?: string
}

const MENU_WIDTH = 160
const MENU_ITEM_HEIGHT = 24
const MENU_VERTICAL_PADDING = 4
const VIEWPORT_MARGIN = 10

const contextMenuState = reactive<ContextMenuState>({
  visible: false,
  x: 0,
  y: 0,
  items: [],
  sourceId: undefined
})

const hideContextMenu = (): void => {
  contextMenuState.visible = false
  contextMenuState.items = []
  contextMenuState.sourceId = undefined
}

const clamp = (value: number, min: number, max: number): number => {
  return Math.min(Math.max(value, min), max)
}

const calculateMenuPosition = (
  clientX: number,
  clientY: number,
  itemCount: number
): { x: number; y: number } => {
  const menuHeight = itemCount * MENU_ITEM_HEIGHT + MENU_VERTICAL_PADDING
  const maxX = window.innerWidth - MENU_WIDTH - VIEWPORT_MARGIN
  const maxY = window.innerHeight - menuHeight - VIEWPORT_MARGIN
  const safeMaxX = Math.max(VIEWPORT_MARGIN, maxX)
  const safeMaxY = Math.max(VIEWPORT_MARGIN, maxY)

  return {
    x: clamp(clientX, VIEWPORT_MARGIN, safeMaxX),
    y: clamp(clientY, VIEWPORT_MARGIN, safeMaxY)
  }
}

const isTargetInsideContextMenu = (target: EventTarget | null): boolean => {
  if (!(target instanceof HTMLElement)) {
    return false
  }

  const mainMenu = document.querySelector('.context-menu:not(.submenu)')
  const submenu = document.querySelector('.context-menu.submenu')

  return Boolean(mainMenu?.contains(target) || submenu?.contains(target))
}

let globalListenersRegistered = false

const registerGlobalListeners = (): void => {
  if (globalListenersRegistered || typeof document === 'undefined') {
    return
  }

  document.addEventListener('click', (event) => {
    if (contextMenuState.visible && !isTargetInsideContextMenu(event.target)) {
      hideContextMenu()
    }
  })

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && contextMenuState.visible) {
      hideContextMenu()
    }
  })

  globalListenersRegistered = true
}

registerGlobalListeners()

export function useContextMenu(): {
  contextMenuState: ContextMenuState
  showContextMenu: (event: MouseEvent, items: ContextMenuItem[], sourceId?: string) => void
  hideContextMenu: () => void
  isMenuFromSource: (sourceId: string) => boolean
} {
  const showContextMenu = (
    event: MouseEvent,
    items: ContextMenuItem[],
    sourceId?: string
  ): void => {
    event.preventDefault()
    event.stopPropagation()

    const { x, y } = calculateMenuPosition(event.clientX, event.clientY, items.length)

    contextMenuState.visible = true
    contextMenuState.x = x
    contextMenuState.y = y
    contextMenuState.items = items
    contextMenuState.sourceId = sourceId
  }

  const isMenuFromSource = (sourceId: string): boolean => {
    return contextMenuState.visible && contextMenuState.sourceId === sourceId
  }

  return {
    contextMenuState,
    showContextMenu,
    hideContextMenu,
    isMenuFromSource
  }
}
