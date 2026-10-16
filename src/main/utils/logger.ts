import log from 'electron-log'
import type { LogFunctions } from 'electron-log'
import { join } from 'path'

/**
 * 日志配置选项
 */
export interface LoggerOptions {
  userDataPath: string
  fileName?: string
  scope?: string
}

/**
 * 日志配置和管理
 */
class Logger {
  private static instance: Logger
  private isInitialized = false

  private constructor() {
    // 不在构造函数中初始化，等待外部配置
  }

  public static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger()
    }
    return Logger.instance
  }

  /**
   * 初始化日志配置
   */
  public initialize(options: LoggerOptions): void {
    if (this.isInitialized) {
      return
    }

    try {
      // 设置日志文件路径
      const fileName = options.fileName || 'main.log'
      log.transports.file.resolvePathFn = () => join(options.userDataPath, 'logs', fileName)

      // 配置文件传输
      log.transports.file.level = 'info'
      // 切换诊断日志异步落盘，避免文件写入阻塞主进程快捷键回调。
      log.transports.file.sync = false
      log.transports.file.maxSize = 10 * 1024 * 1024 // 10MB
      log.transports.file.format = '[{y}-{m}-{d} {h}:{i}:{s}.{ms}] [{level}] {text}'

      // 配置控制台传输
      log.transports.console.level = 'debug'
      log.transports.console.format = '[{y}-{m}-{d} {h}:{i}:{s}.{ms}] [{level}] {text}'

      // 在开发环境中显示更详细的日志
      if (process.env.NODE_ENV === 'development') {
        log.transports.console.level = 'silly'
        log.transports.file.level = 'debug'
      }

      // 设置日志范围
      if (options.scope) {
        log.scope(options.scope)
      }

      this.isInitialized = true
    } catch (error) {
      console.error('[日志系统] 初始化失败:', error)
    }
  }

  /**
   * 获取日志实例
   */
  public getLogger(): typeof log {
    return log
  }

  /**
   * 创建带作用域的日志器
   */
  public createScopedLogger(scope: string): LogFunctions {
    return log.scope(scope)
  }

  /**
   * 记录错误信息
   */
  public error(message: string, ...args: any[]): void {
    log.error(message, ...args)
  }

  /**
   * 记录警告信息
   */
  public warn(message: string, ...args: any[]): void {
    log.warn(message, ...args)
  }

  /**
   * 记录信息
   */
  public info(message: string, ...args: any[]): void {
    log.info(message, ...args)
  }

  /**
   * 记录调试信息
   */
  public debug(message: string, ...args: any[]): void {
    log.debug(message, ...args)
  }

  /**
   * 记录详细信息
   */
  public verbose(message: string, ...args: any[]): void {
    log.verbose(message, ...args)
  }

  /**
   * 记录最详细信息
   */
  public silly(message: string, ...args: any[]): void {
    log.silly(message, ...args)
  }
}

// 导出单例实例
export default Logger.getInstance()
export { Logger }
