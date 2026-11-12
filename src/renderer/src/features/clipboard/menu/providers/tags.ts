import { tags } from '@renderer/api/renderer-api'
import { showMessage } from '@renderer/api/dialog'
import type { ContextMenuItem } from '@renderer/composables/useContextMenu'
import { PREVIEW_ACTIONS } from '@renderer/constants/clipboard-preview'
import IconTag from '~icons/mdi/tag'
import IconCheck from '~icons/mdi/check'
import IconCircle from '~icons/mdi/circle'
import IconDelete from '~icons/mdi/delete'
import type { ClipboardMenuProvider } from '../types'
import { separator } from '../separator'

const updateTag = async (itemId: string, tagId: string, remove: boolean): Promise<void> => {
  const operation = remove ? '解绑' : '绑定'
  try {
    if (remove) {
      await tags.unbindTagFromRecord(itemId, tagId)
    } else {
      await tags.bindTagToRecord(itemId, tagId)
    }
  } catch (error) {
    console.error(`${operation}标签失败:`, error)
    await showMessage(`${operation}标签失败: ` + (error as Error).message)
  }
}

export const tagsMenu: ClipboardMenuProvider = {
  matches: ({ allTags }) => Boolean(allTags?.length),
  createItems: ({ item, allTags = [], activeTagId }) => {
    const currentTagIds = new Set(item.tags?.map((tag) => tag.id))
    const submenu: ContextMenuItem[] = allTags.map((tag) => {
      const isTagged = currentTagIds.has(tag.id)
      return {
        label: tag.name,
        icon: isTagged ? IconCheck : IconCircle,
        iconColor: tag.color,
        action: () => updateTag(item.id, tag.id, isTagged)
      }
    })
    if (activeTagId !== 'all' && currentTagIds.size > 0) {
      submenu.push(separator(), {
        label: PREVIEW_ACTIONS.removeCurrentTag,
        icon: IconDelete,
        action: () => {
          if (activeTagId) return updateTag(item.id, activeTagId, true)
        }
      })
    }
    return [
      {
        label: PREVIEW_ACTIONS.manageTags,
        icon: IconTag,
        action: () => undefined,
        submenu
      }
    ]
  }
}
