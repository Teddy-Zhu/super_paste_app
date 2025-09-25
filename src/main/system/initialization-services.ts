import databaseService from '../services/database-service'
import autoStartManager from '../utils/auto-start-manager'
import { STORE_KEY } from '../config/common'
import store from '../config/store'
import logger from '../utils/logger'

/**
 * 数据初始化服务
 */
export class DataInitializationService {
  /**
   * 创建示例标签数据（如果不存在）
   */
  static async createSampleTagsIfNeeded(): Promise<void> {
    try {
      logger.debug('[主进程] 检查示例标签数据')

      // 检查是否已有标签数据
      const existingTags = databaseService.getAllTagGroups()

      if (existingTags.length === 0) {
        logger.debug('[主进程] 未找到标签数据，创建示例标签')

        // 创建一些示例标签
        const sampleTags = [
          { name: 'Work', color: '#3b82f6', description: 'Work related content' },
          { name: 'Personal', color: '#10b981', description: 'Personal notes and content' },
          { name: 'Code', color: '#8b5cf6', description: 'Code snippets and programming' },
          { name: 'Links', color: '#f59e0b', description: 'Useful links and URLs' },
          { name: 'Ideas', color: '#ef4444', description: 'Creative ideas and inspiration' }
        ]

        for (const tag of sampleTags) {
          try {
            const tagId = databaseService.createTagGroup({
              name: tag.name,
              color: tag.color,
              description: tag.description
            })

            if (!tagId) {
              logger.error(`[主进程] 创建标签失败: ${tag.name}`)
            } else {
              logger.debug(`[主进程] 创建标签成功: ${tag.name}`)
            }
          } catch (error) {
            logger.error(`[主进程] 创建标签失败: ${tag.name}`, error)
          }
        }

        logger.debug('[主进程] 示例标签创建完成')
      } else {
        logger.debug(`[主进程] 已存在 ${existingTags.length} 个标签，跳过示例数据创建`)
      }
    } catch (error) {
      logger.error('[主进程] 创建示例标签数据失败:', error)
    }
  }
}

/**
 * 系统集成服务
 */
export class SystemIntegrationService {
  /**
   * 同步自启状态
   * 检查配置中的自启设置与系统实际状态是否一致，如果不一致则同步
   */
  static async syncAutoStartStatus(): Promise<void> {
    try {
      logger.debug('[主进程] 开始同步自启状态')

      // 获取配置中的自启状态
      const configuredAutoStart = store.get(STORE_KEY.SETTINGS_AUTO_START, false) as boolean

      // 获取系统中的实际自启状态
      const actualAutoStart = await autoStartManager.isEnabled()

      logger.debug(`[主进程] 自启状态检查 - 配置: ${configuredAutoStart}, 实际: ${actualAutoStart}`)

      // 如果状态不一致，则根据配置更新系统状态
      if (configuredAutoStart !== actualAutoStart) {
        logger.debug(`[主进程] 自启状态不一致，正在同步到配置状态: ${configuredAutoStart}`)

        const success = await autoStartManager.setEnabled(configuredAutoStart)

        if (success) {
          logger.debug('[主进程] 自启状态同步成功')
        } else {
          logger.error('[主进程] 自启状态同步失败')
        }
      } else {
        logger.debug('[主进程] 自启状态已同步')
      }
    } catch (error) {
      logger.error('[主进程] 同步自启状态失败:', error)
    }
  }

  /**
   * 初始化系统集成相关功能
   */
  static async initialize(): Promise<void> {
    try {
      logger.debug('[主进程] 开始初始化系统集成服务')

      // 1. 创建示例标签数据
      await DataInitializationService.createSampleTagsIfNeeded()

      // 2. 同步自启状态
      await this.syncAutoStartStatus()

      logger.debug('[主进程] 系统集成服务初始化完成')
    } catch (error) {
      logger.error('[主进程] 系统集成服务初始化失败:', error)
      throw error
    }
  }
}
