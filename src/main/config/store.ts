import Store from 'electron-store'
import { BrowserWindow } from 'electron'
import logger from '../utils/logger'
import { isDeepStrictEqual } from 'node:util'
import type { StoreMutationContext, StoreSnapshot, StoreValueChange } from '../../shared/store-sync'

export { Store }

/**
 * 加密的 Electron Store 单例类
 * 使用 electron-store 内置的加密功能
 */
class EncryptedStore {
  private static instance: EncryptedStore
  private store: Store<Record<string, unknown>>
  private version = 0
  private readonly fieldVersions = new Map<string, number>()

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
  public get<T = unknown>(key: string, defaultValue?: T): T {
    try {
      return this.store.get(key, defaultValue) as T
    } catch (error) {
      logger.error(`读取数据失败 [${key}]:`, error)
      return defaultValue as T
    }
  }

  public getFieldVersion(key: string): number {
    return this.fieldVersions.get(key.split('.').slice(0, 2).join('.')) ?? 0
  }

  public getSnapshot(key: string): StoreSnapshot {
    return { value: this.store.get(key) as StoreSnapshot['value'], version: this.version }
  }

  public set<T = unknown>(key: string, value: T, context?: StoreMutationContext): void {
    this.setMultiple({ [key]: value }, context)
  }

  // 同步以 Pinia 顶层字段为单位，嵌套写入和整组替换使用同一条广播链路。
  private publishRootChanges(
    before: Record<string, unknown>,
    context: StoreMutationContext = { source: { kind: 'main', id: 'config' } }
  ): void {
    for (const [root, previous] of Object.entries(before)) {
      const current = this.store.get(root)
      const isObject = (value: unknown): value is Record<string, unknown> =>
        value !== null && typeof value === 'object' && !Array.isArray(value)
      const fields = new Set([
        ...(isObject(previous) ? Object.keys(previous) : []),
        ...(isObject(current) ? Object.keys(current) : [])
      ])
      const changes = fields.size
        ? [...fields].map((field) => ({
            key: `${root}.${field}`,
            previous: isObject(previous) ? previous[field] : undefined,
            value: isObject(current) ? current[field] : undefined
          }))
        : [{ key: root, previous, value: current }]
      for (const change of changes) {
        if (isDeepStrictEqual(change.previous, change.value)) continue
        const event: StoreValueChange = {
          key: change.key,
          value: change.value,
          ...context,
          version: ++this.version,
          timestamp: Date.now()
        }
        this.fieldVersions.set(change.key, event.version)
        for (const window of BrowserWindow.getAllWindows()) {
          if (window.isDestroyed() || window.webContents.isDestroyed()) continue
          try {
            window.webContents.send('store-value-changed', event)
          } catch (error) {
            logger.error('广播存储变更事件失败:', error)
          }
        }
      }
    }
  }

  public delete(key: string, context?: StoreMutationContext): void {
    const root = key.split('.')[0]
    const before = { [root]: this.store.get(root) }
    this.store.delete(key)
    this.publishRootChanges(before, context)
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
      const before = this.store.store
      this.store.clear()
      this.publishRootChanges(before)
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
  public setMultiple(data: Record<string, unknown>, context?: StoreMutationContext): void {
    const changes = Object.fromEntries(
      Object.entries(data).filter(([key, value]) => !isDeepStrictEqual(this.store.get(key), value))
    )
    if (!Object.keys(changes).length) return
    const before = Object.fromEntries(
      [...new Set(Object.keys(changes).map((key) => key.split('.')[0]))].map((root) => [
        root,
        this.store.get(root)
      ])
    )
    // electron-store 的对象写入只序列化和落盘一次，成功后才发布变更。
    this.store.set(changes as Parameters<typeof this.store.set>[0])
    this.publishRootChanges(before, context)
  }

  /**
   * 批量获取数据
   * @param keys 要获取的键名数组
   * @returns 包含所有键值对的对象
   */
  public getMultiple(keys: string[]): Record<string, unknown> {
    try {
      const result: Record<string, unknown> = {}
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
    this.clear()
  }
}

export default EncryptedStore.getInstance()
