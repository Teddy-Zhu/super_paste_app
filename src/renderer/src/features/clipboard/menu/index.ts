import type { ContextMenuItem } from '@renderer/composables/useContextMenu'
import type { ClipboardMenuContext, ClipboardMenuProvider } from './types'
import { commonMenu, deleteMenu } from './providers/common'
import { colorMenu } from './providers/color'
import { imageMenu } from './providers/image'
import { textMenu } from './providers/text'
import { linkMenu } from './providers/link'
import { tagsMenu } from './providers/tags'

// 注册顺序即展示顺序；类型模块自行管理适用条件与操作。
const providers: readonly ClipboardMenuProvider[] = [
  commonMenu,
  colorMenu,
  imageMenu,
  textMenu,
  linkMenu,
  tagsMenu,
  deleteMenu
]

export function createClipboardMenu(context: ClipboardMenuContext): ContextMenuItem[] {
  return providers
    .filter((provider) => provider.matches(context))
    .flatMap((provider) => provider.createItems(context))
}
