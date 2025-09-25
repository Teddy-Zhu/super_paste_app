import { reactive } from 'vue'

interface MenuItem {
  label: string
  action: () => void
  icon?: string | any // 支持字符串或组件
  iconColor?: string // 支持图标颜色
  disabled?: boolean
  submenu?: MenuItem[] // 支持二级菜单
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

/**
 * 全局上下文菜单管理器
 */
export function useContextMenu() {
  /**
   * 显示上下文菜单
   */
  const showContextMenu = (event: MouseEvent, items: MenuItem[], sourceId?: string) => {
    // 阻止默认右键菜单
    event.preventDefault()
    event.stopPropagation()

    // 获取真实的鼠标位置（考虑元素变换）
    const { x: realX, y: realY } = getRealMousePosition(event)

    // 计算菜单位置，确保不超出屏幕边界
    const { x, y } = calculateMenuPosition(realX, realY, items.length)

    // 更新全局状态
    contextMenuState.visible = true
    contextMenuState.x = x
    contextMenuState.y = y
    contextMenuState.items = items
    contextMenuState.sourceId = sourceId
  }

  /**
   * 隐藏上下文菜单
   */
  const hideContextMenu = () => {
    contextMenuState.visible = false
    contextMenuState.sourceId = undefined
  }

  /**
   * 获取真实的鼠标位置（考虑元素变换）
   */
  const getRealMousePosition = (event: MouseEvent) => {
    // 对于 fixed 定位的菜单，直接使用 clientX/clientY
    // 这些坐标是相对于视口的，已经考虑了所有变换
    const x = event.clientX
    const y = event.clientY

    // 调试信息
    console.log('[ContextMenu] 鼠标位置:', { x, y })

    return { x, y }
  }

  /**
   * 计算菜单位置，确保不超出屏幕边界
   */
  const calculateMenuPosition = (clientX: number, clientY: number, itemCount: number) => {
    const menuWidth = 160 // 菜单宽度
    const menuHeight = itemCount * 24 + 4 // 每个菜单项24px高度 + 4px padding (上下各2px)
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

    // 调试信息
    console.log('[ContextMenu] 菜单位置计算:', {
      原始坐标: { x: clientX, y: clientY },
      计算后坐标: { x, y },
      菜单尺寸: { width: menuWidth, height: menuHeight },
      屏幕尺寸: { width: screenWidth, height: screenHeight }
    })

    return { x, y }
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

// 全局点击事件监听器
document.addEventListener('click', (event) => {
  const { contextMenuState, hideContextMenu } = useContextMenu()

  if (contextMenuState.visible) {
    // 检查点击是否在菜单外部
    const target = event.target as HTMLElement
    const mainMenuElement = document.querySelector('.context-menu:not(.submenu)')
    const submenuElement = document.querySelector('.context-menu.submenu')

    // 如果点击在主菜单或二级菜单内，不关闭菜单
    const isInMainMenu = mainMenuElement && mainMenuElement.contains(target)
    const isInSubmenu = submenuElement && submenuElement.contains(target)

    if (!isInMainMenu && !isInSubmenu) {
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
