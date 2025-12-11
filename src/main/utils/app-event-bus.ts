import { EventEmitter } from 'node:events'
import logger from '../utils/logger'

/**
 * 应用事件总线
 * 用于应用内部模块间的事件通信
 */
class AppEventBus extends EventEmitter {
    private static instance: AppEventBus

    private constructor() {
        super()
        // 设置最大监听器数量,避免内存泄漏警告
        this.setMaxListeners(20)
    }

    /**
     * 获取单例实例
     */
    public static getInstance(): AppEventBus {
        if (!AppEventBus.instance) {
            AppEventBus.instance = new AppEventBus()
        }
        return AppEventBus.instance
    }

    /**
     * 发送应用退出请求
     */
    public requestAppQuit(): void {
        logger.info('[事件总线] 发送应用退出请求')
        this.emit('app:quit-requested')
    }

    /**
     * 监听应用退出请求
     */
    public onQuitRequested(handler: () => void | Promise<void>): void {
        this.on('app:quit-requested', handler)
    }

    /**
     * 移除退出请求监听器
     */
    public offQuitRequested(handler: () => void | Promise<void>): void {
        this.off('app:quit-requested', handler)
    }
}

// 导出单例实例
export const appEventBus = AppEventBus.getInstance()
export default appEventBus
