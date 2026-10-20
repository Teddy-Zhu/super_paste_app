/**
 * IPC 渲染进程服务封装
 * 只暴露项目中实际使用的 IPC 方法，提高安全性和性能
 */

import type { CustomElectronAPI, IpcRendererListener } from '@preload/index.d'
import type { IpcResponses } from './types'
type SimpleListener<Args extends unknown[]> = (...args: Args) => void

/**
 * 获取 ipcRenderer 实例
 */
function getIpcRenderer(): CustomElectronAPI['ipcRenderer'] {
  if (!window.electron?.ipcRenderer) {
    throw new Error('ipcRenderer 不可用，请确保在 Electron 环境中运行')
  }
  return window.electron.ipcRenderer
}

/**
 * 监听指定频道的消息
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function on<Args extends unknown[]>(
  channel: string,
  listener: IpcRendererListener<Args>
): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.on(channel, listener)
}

/**
 * 监听指定频道的消息（简化版，不包含 event 参数）
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function onSimple<Args extends unknown[]>(
  channel: string,
  listener: SimpleListener<Args>
): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.on(channel, (_, ...args: Args) => listener(...args))
}

/**
 * 添加一次性监听器
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function once<Args extends unknown[]>(
  channel: string,
  listener: IpcRendererListener<Args>
): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.once(channel, listener)
}

/**
 * 添加一次性监听器（简化版，不包含 event 参数）
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function onceSimple<Args extends unknown[]>(
  channel: string,
  listener: SimpleListener<Args>
): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.once(channel, (_, ...args: Args) => listener(...args))
}

/**
 * 移除指定频道的指定监听器
 * @param channel 频道名称
 * @param listener 要移除的监听器函数
 * @deprecated 推荐使用 on() 方法返回的取消函数
 */
export function removeListener<Args extends unknown[]>(
  channel: string,
  listener: IpcRendererListener<Args>
): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.removeListener(channel, listener)
}

/**
 * 移除指定频道的所有监听器
 * @param channel 频道名称
 */
export function removeAllListeners(channel: string): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.removeAllListeners(channel)
}

/**
 * 向主进程发送异步消息
 * @param channel 频道名称
 * @param args 要发送的参数
 */
export function send(channel: string, ...args: unknown[]): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.send(channel, ...args)
}

/**
 * 向主进程发送消息并等待响应
 * @param channel 频道名称
 * @param args 要发送的参数
 * @returns Promise，解析为主进程的响应
 */
export function invoke<K extends keyof IpcResponses>(
  channel: K,
  ...args: unknown[]
): Promise<IpcResponses[K]>
export function invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T>
export function invoke<T = unknown>(channel: string, ...args: unknown[]): Promise<T> {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.invoke(channel, ...args) as Promise<T>
}

/**
 * 向主进程发送同步消息（仅在必要时使用，如 Pinia 持久化存储）
 * @param channel 频道名称
 * @param args 要发送的参数
 * @returns 主进程的同步响应
 */
export function sendSync<T = unknown>(channel: string, ...args: unknown[]): T {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.sendSync(channel, ...args) as T
}

// 默认导出实际使用的方法
export default {
  on,
  onSimple,
  once,
  onceSimple,
  /** @deprecated 推荐使用 on() 方法返回的取消函数 */
  removeListener,
  removeAllListeners,
  send,
  invoke,
  sendSync
}
