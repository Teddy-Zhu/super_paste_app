/**
 * Worker 线程专用日志工具
 *
 * Worker 线程运行在纯 Node.js 环境中，无法访问 electron 模块。
 * 因此使用 electron-log/node 代替 electron-log，避免打包后出现
 * "Cannot find module 'electron'" 错误。
 */
import log from 'electron-log/node'
import type { LogFunctions } from 'electron-log'
import { join } from 'path'

export interface WorkerLoggerOptions {
    userDataPath: string
    fileName?: string
    scope?: string
}

class WorkerLogger {
    private static instance: WorkerLogger
    private isInitialized = false

    private constructor() { }

    public static getInstance(): WorkerLogger {
        if (!WorkerLogger.instance) {
            WorkerLogger.instance = new WorkerLogger()
        }
        return WorkerLogger.instance
    }

    public initialize(options: WorkerLoggerOptions): void {
        if (this.isInitialized) {
            return
        }

        try {
            const fileName = options.fileName || 'worker.log'
            log.transports.file.resolvePathFn = () => join(options.userDataPath, 'logs', fileName)

            log.transports.file.level = 'info'
            log.transports.file.maxSize = 10 * 1024 * 1024 // 10MB
            log.transports.file.format = '[{y}-{m}-{d} {h}:{i}:{s}.{ms}] [{level}] {text}'

            log.transports.console.level = 'debug'
            log.transports.console.format = '[{y}-{m}-{d} {h}:{i}:{s}.{ms}] [{level}] {text}'

            if (process.env.NODE_ENV === 'development') {
                log.transports.console.level = 'silly'
                log.transports.file.level = 'debug'
            }

            if (options.scope) {
                log.scope(options.scope)
            }

            this.isInitialized = true
        } catch (error) {
            console.error('[Worker 日志系统] 初始化失败:', error)
        }
    }

    public getLogger(): typeof log {
        return log
    }

    public createScopedLogger(scope: string): LogFunctions {
        return log.scope(scope)
    }

    public error(message: string, ...args: any[]): void {
        log.error(message, ...args)
    }

    public warn(message: string, ...args: any[]): void {
        log.warn(message, ...args)
    }

    public info(message: string, ...args: any[]): void {
        log.info(message, ...args)
    }

    public debug(message: string, ...args: any[]): void {
        log.debug(message, ...args)
    }

    public verbose(message: string, ...args: any[]): void {
        log.verbose(message, ...args)
    }

    public silly(message: string, ...args: any[]): void {
        log.silly(message, ...args)
    }
}

export default WorkerLogger.getInstance()
export { WorkerLogger }
