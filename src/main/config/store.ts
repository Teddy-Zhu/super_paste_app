import __Store from 'electron-store'

//@ts-ignore
export const Store = __Store.default || __Store
import { BrowserWindow } from 'electron'
import logger from '../utils/logger'

/**
 * 加密的 Electron Store 单例类
 * 使用 electron-store 内置的加密功能
 */
class EncryptedStore {
  private static instance: EncryptedStore
  private store: any

  private constructor() {
    // 使用 electron-store 内置加密功能
    this.store = new Store({
      // 启用加密，electron-store 会自动处理加密/解密
      encryptionKey: this.generateEncryptionKey()
    })
  }

  /**
   * 获取单例实例
   */
  public static getInstance(): EncryptedStore {
    if (!EncryptedStore.instance) {
      EncryptedStore.instance = new EncryptedStore()
    }
    return EncryptedStore.instance
  }

  /**
   * 生成加密密钥
   * 基于应用信息生成一致的密钥
   */
  private generateEncryptionKey(): string {
    return 'super_paste_app_11'
  }

  /**
   * 读取数据
   * @param key 键名
   * @param defaultValue 默认值
   * @returns 存储的值或默认值
   */
  public get<T = any>(key: string, defaultValue?: T): T {
    try {
      return this.store.get(key, defaultValue) as T
    } catch (error) {
      logger.error(`读取数据失败 [${key}]:`, error)
      return defaultValue as T
    }
  }

  public raw_set<T = any>(key: string, value: T): void {
    try {
      this.store.set(key, value)
    } catch (error) {
      logger.error(`写入数据失败 [${key}]:`, error)
      throw error
    }
  }

  /**
   * 写入数据
   * @param key 键名
   * @param value 要存储的值
   */
  public set<T = any>(key: string, value: T): void {
    try {
      this.store.set(key, value)

      // 发送 store-value-changed 事件到所有渲染进程
      this.broadcastStoreChange(key, value)
    } catch (error) {
      logger.error(`写入数据失败 [${key}]:`, error)
      throw error
    }
  }

  /**
   * 向所有渲染进程广播存储值变更事件
   * @param key 变更的键名
   * @param value 新的值
   */
  private broadcastStoreChange<T = any>(key: string, value: T): void {
    try {
      // 获取所有窗口并发送事件
      const allWindows = BrowserWindow.getAllWindows()
      allWindows.forEach((window) => {
        if (window && !window.isDestroyed()) {
          window.webContents.send('store-value-changed', {
            key,
            value,
            timestamp: Date.now()
          })
        }
      })
    } catch (error) {
      logger.error('广播存储变更事件失败:', error)
    }
  }

  /**
   * 删除数据
   * @param key 键名
   */
  public delete(key: string): void {
    try {
      this.store.delete(key)
    } catch (error) {
      logger.error(`删除数据失败 [${key}]:`, error)
      throw error
    }
  }

  /**
   * 检查键是否存在
   * @param key 键名
   * @returns 是否存在
   */
  public has(key: string): boolean {
    try {
      return this.store.has(key)
    } catch (error) {
      logger.error(`检查键存在性失败 [${key}]:`, error)
      return false
    }
  }

  /**
   * 清空所有数据
   */
  public clear(): void {
    try {
      this.store.clear()
    } catch (error) {
      logger.error('清空数据失败:', error)
      throw error
    }
  }

  /**
   * 获取所有键名
   * @returns 所有键名数组
   */
  public getAllKeys(): string[] {
    try {
      return Object.keys(this.store.store)
    } catch (error) {
      logger.error('获取所有键名失败:', error)
      return []
    }
  }

  /**
   * 获取存储文件大小（字节）
   * @returns 文件大小
   */
  public getSize(): number {
    try {
      return this.store.size
    } catch (error) {
      logger.error('获取存储大小失败:', error)
      return 0
    }
  }

  /**
   * 获取存储文件路径
   * @returns 文件路径
   */
  public getPath(): string {
    try {
      return this.store.path
    } catch (error) {
      logger.error('获取存储路径失败:', error)
      return ''
    }
  }

  /**
   * 批量设置数据
   * @param data 要设置的数据对象
   */
  public setMultiple(data: Record<string, any>): void {
    try {
      for (const [key, value] of Object.entries(data)) {
        this.store.set(key, value)
        // 为每个设置的键值对发送变更事件
        this.broadcastStoreChange(key, value)
      }
    } catch (error) {
      logger.error('批量设置数据失败:', error)
      throw error
    }
  }

  /**
   * 批量获取数据
   * @param keys 要获取的键名数组
   * @returns 包含所有键值对的对象
   */
  public getMultiple(keys: string[]): Record<string, any> {
    try {
      const result: Record<string, any> = {}
      for (const key of keys) {
        if (this.store.has(key)) {
          result[key] = this.store.get(key)
        }
      }
      return result
    } catch (error) {
      logger.error('批量获取数据失败:', error)
      return {}
    }
  }

  /**
   * 重置存储（清空所有数据）
   */
  public reset(): void {
    try {
      this.store.clear()
    } catch (error) {
      logger.error('重置存储失败:', error)
      throw error
    }
  }

  /**
   * 获取原始 store 实例（用于高级操作）
   */
  public getStore(): any {
    return this.store
  }
}

// 导出单例实例
export default EncryptedStore.getInstance()
