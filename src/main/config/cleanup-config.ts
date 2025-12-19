import store from './store'
import { STORE_KEY } from './common'
import type { CleanupConfig } from '../features/clipboard/types/clipboard-types'

/**
 * 清理配置管理工具类
 * 统一管理剪贴板历史清理相关的配置读取和创建
 */
export class CleanupConfigManager {
    /**
     * 从 store 读取清理配置
     */
    static getCleanupConfig(): CleanupConfig {
        const maxKeepTime = store.get(STORE_KEY.SETTINGS_MAX_KEEP_TIME, -1) as number
        const maxHistorySize = store.get(STORE_KEY.SETTINGS_MAX_HISTORY_SIZE, 100) as number

        return {
            maxKeepTime,
            maxHistorySize,
            cleanupInterval: 60 * 60 * 1000, // 1小时
            enableAutoCleanup: maxKeepTime !== -1,
            enableCountBasedCleanup: true
        }
    }

    /**
     * 创建初始化配置（用于 Worker 初始化）
     */
    static createInitialConfig(): Partial<CleanupConfig> {
        return this.getCleanupConfig()
    }
}

// 导出便捷方法
export const getCleanupConfig = CleanupConfigManager.getCleanupConfig.bind(CleanupConfigManager)
export const createInitialConfig = CleanupConfigManager.createInitialConfig.bind(CleanupConfigManager)
