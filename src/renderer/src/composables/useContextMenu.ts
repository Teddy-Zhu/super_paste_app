import { reactive } from 'vue'

interface MenuItem {
  label: string
  action: () => void
  icon?: string
  disabled?: boolean
}

interface ContextMenuState {
  visible: boolean
  x: number
  y: number
  items: MenuItem[]
  sourceId?: string // 用于标识菜单来源
}

// 全局上下文菜单状态
const contextMenuState = reactive<ContextMenuState>({
  visible: false,
  x: 0,
  y: 0,
  items: [],
  sourceId: undefined
})

// 鼠标移动计时器
let mouseMoveTimer: number | null = null
let lastMouseX = 0
let lastMouseY = 0

/**
 * 全局上下文菜单管理器
 */
export function useContextMenu() {
  
  /**
   * 显示上下文菜单
   */
  const showContextMenu = (
    event: MouseEvent, 
    items: MenuItem[], 
    sourceId?: string
  ) => {
    // 阻止默认右键菜单
    event.preventDefault()
    event.stopPropagation()

    // 计算菜单位置，确保不超出屏幕边界
    const { x, y } = calculateMenuPosition(event.clientX, event.clientY, items.length)

    // 更新全局状态
    contextMenuState.visible = true
    contextMenuState.x = x
    contextMenuState.y = y
    contextMenuState.items = items
    contextMenuState.sourceId = sourceId

    // 记录当前鼠标位置
    lastMouseX = event.clientX
    lastMouseY = event.clientY

    // 开始监听鼠标移动
    startMouseMoveDetection()
  }

  /**
   * 隐藏上下文菜单
   */
  const hideContextMenu = () => {
    contextMenuState.visible = false
    contextMenuState.sourceId = undefined
    stopMouseMoveDetection()
  }

  /**
   * 计算菜单位置，确保不超出屏幕边界
   */
  const calculateMenuPosition = (clientX: number, clientY: number, itemCount: number) => {
    const menuWidth = 160 // 菜单宽度
    const menuHeight = itemCount * 40 + 8 // 每个菜单项40px高度 + 8px padding
    const margin = 10 // 边距

    const screenWidth = window.innerWidth
    const screenHeight = window.innerHeight

    let x = clientX
    let y = clientY

    // 检查右边界 - 如果菜单会超出右边界，则显示在鼠标左侧
    if (x + menuWidth > screenWidth - margin) {
      x = Math.max(margin, clientX - menuWidth)
    }

    // 检查下边界 - 如果菜单会超出下边界，则显示在鼠标上方
    if (y + menuHeight > screenHeight - margin) {
      y = Math.max(margin, clientY - menuHeight)
    }

    // 确保不超出左边界和上边界
    x = Math.max(margin, x)
    y = Math.max(margin, y)

    // 最终检查，确保菜单完全在屏幕内
    if (x + menuWidth > screenWidth - margin) {
      x = screenWidth - menuWidth - margin
    }
    if (y + menuHeight > screenHeight - margin) {
      y = screenHeight - menuHeight - margin
    }

    return { x, y }
  }

  /**
   * 开始鼠标移动检测
   */
  const startMouseMoveDetection = () => {
    stopMouseMoveDetection() // 清除之前的计时器

    let mouseMoveHandler: ((event: MouseEvent) => void) | null = null

    const handleMouseMove = (event: MouseEvent) => {
      const deltaX = Math.abs(event.clientX - lastMouseX)
      const deltaY = Math.abs(event.clientY - lastMouseY)

      // 如果鼠标在菜单区域内，不隐藏菜单
      if (isMouseInMenuArea(event.clientX, event.clientY)) {
        return
      }

      // 如果鼠标移动超过阈值，隐藏菜单
      if (deltaX > 8 || deltaY > 8) {
        hideContextMenu()
        if (mouseMoveHandler) {
          document.removeEventListener('mousemove', mouseMoveHandler)
          mouseMoveHandler = null
        }
      }
    }

    mouseMoveHandler = handleMouseMove

    // 延迟添加鼠标移动监听，避免立即触发
    mouseMoveTimer = window.setTimeout(() => {
      if (mouseMoveHandler && contextMenuState.visible) {
        document.addEventListener('mousemove', mouseMoveHandler)
      }
    }, 150)
  }

  /**
   * 停止鼠标移动检测
   */
  const stopMouseMoveDetection = () => {
    if (mouseMoveTimer) {
      clearTimeout(mouseMoveTimer)
      mouseMoveTimer = null
    }
  }

  /**
   * 检查菜单是否属于指定来源
   */
  const isMenuFromSource = (sourceId: string) => {
    return contextMenuState.visible && contextMenuState.sourceId === sourceId
  }

  return {
    contextMenuState,
    showContextMenu,
    hideContextMenu,
    isMenuFromSource
  }
}

// 检查鼠标是否在菜单区域内
const isMouseInMenuArea = (mouseX: number, mouseY: number) => {
  if (!contextMenuState.visible) return false

  const menuWidth = 160
  const menuHeight = contextMenuState.items.length * 40 + 8
  const padding = 10 // 额外的容错区域

  return mouseX >= contextMenuState.x - padding &&
         mouseX <= contextMenuState.x + menuWidth + padding &&
         mouseY >= contextMenuState.y - padding &&
         mouseY <= contextMenuState.y + menuHeight + padding
}

// 全局点击事件监听器
document.addEventListener('click', (event) => {
  const { contextMenuState, hideContextMenu } = useContextMenu()

  if (contextMenuState.visible) {
    // 检查点击是否在菜单外部
    const target = event.target as HTMLElement
    const menuElement = document.querySelector('.context-menu')

    if (!menuElement || !menuElement.contains(target)) {
      hideContextMenu()
    }
  }
})

// 全局ESC键监听器
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    const { hideContextMenu } = useContextMenu()
    hideContextMenu()
  }
})
