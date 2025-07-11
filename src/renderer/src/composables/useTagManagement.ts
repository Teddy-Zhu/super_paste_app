/**
 * 标签管理 Composable
 * 管理标签数据、统计和切换逻辑
 */

import { ref, computed } from 'vue'
import { tags } from '@renderer/api/renderer-api'
import type { TagGroup, ClipboardHistoryQueryOptions } from '@renderer/api/types'
import type { TagManager } from '@renderer/types/overlay'

export function useTagManagement(
  loadClipboardHistory: (options?: ClipboardHistoryQueryOptions) => Promise<void>
): TagManager {
  // 标签数据
  const allTags = ref<TagGroup[]>([])
  const activeTagId = ref<string>('all') // 默认选中'all'标签
  const isTagSwitching = ref(false)
  
  // 标签统计数据
  const tagStats = ref<Record<string, number>>({})

  // 计算属性：生成标签组件数据，包含'all'标签和实际标签
  const tabs = computed(() => {
    const allTab = {
      id: 'all',
      label: 'All',
      active: activeTagId.value === 'all',
      color: '#10b981',
      count: 0 // All标签不显示数量
    }
    const tagTabs = allTags.value.map(tag => ({
      id: tag.id,
      label: tag.name,
      active: activeTagId.value === tag.id,
      color: tag.color,
      count: tagStats.value[tag.id] || 0
    }))
    return [allTab, ...tagTabs]
  })

  /**
   * 加载所有标签
   */
  async function loadAllTags() {
    try {
      const tagList = await tags.getAllTags()
      allTags.value = tagList
      console.log('[前端] 标签数据加载完成:', tagList)

      // 加载标签统计数据
      await loadTagStats()
    } catch (error) {
      console.error('[前端] 加载标签数据失败:', error)
      allTags.value = []
    }
  }

  /**
   * 加载标签统计数据
   */
  async function loadTagStats() {
    try {
      // 这里可以调用后端API获取每个标签的记录数量
      // 暂时使用模拟数据，实际应该从后端获取
      const stats: Record<string, number> = {}

      for (const tag of allTags.value) {
        // 模拟统计数据，实际应该调用后端API
        stats[tag.id] = Math.floor(Math.random() * 20) + 1
      }

      tagStats.value = stats
      // console.log('[前端] 标签统计数据加载完成:', stats)
    } catch (error) {
      console.error('[前端] 加载标签统计数据失败:', error)
      tagStats.value = {}
    }
  }

  /**
   * 切换标签
   */
  async function switchTag(tagId: string) {
    if (activeTagId.value === tagId || isTagSwitching.value) return

    isTagSwitching.value = true
    activeTagId.value = tagId
    console.log('[前端] 切换到标签:', tagId)

    try {
      // 根据选中的标签重新加载剪贴板历史
      if (tagId === 'all') {
        // 显示所有记录
        await loadClipboardHistory()
      } else {
        // 按标签筛选
        await loadClipboardHistory({
          tagIds: [tagId],
          limit: 50,
          orderBy: 'last_used',
          orderDirection: 'DESC'
        })
      }
    } catch (error) {
      console.error('[前端] 切换标签失败:', error)
    } finally {
      isTagSwitching.value = false
    }
  }

  /**
   * 重新加载标签数据（用于响应更新事件）
   */
  async function reloadTags() {
    try {
      await loadAllTags()
    } catch (error) {
      console.warn('[前端] 重新加载标签数据失败:', error)
    }
  }

  return {
    // 状态
    allTags,
    activeTagId,
    isTagSwitching,
    tagStats,
    tabs,

    // 方法
    loadAllTags,
    loadTagStats,
    switchTag,
    reloadTags
  }
}
