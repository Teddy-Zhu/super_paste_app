import type { IpcRendererEvent } from 'electron'

/**
 * IPC 渲染进程服务封装
 * 对 ipcRenderer 的 on, removeListener, removeAllListeners, once, send, invoke 方法进行封装
 */

// 类型定义
type IpcRendererListener = (event: IpcRendererEvent, ...args: any[]) => void
type SimpleListener = (...args: any[]) => void

/**
 * 获取 ipcRenderer 实例
 */
function getIpcRenderer() {
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
export function on(channel: string, listener: IpcRendererListener): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.on(channel, listener)
}

/**
 * 监听指定频道的消息（简化版，不包含 event 参数）
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function onSimple(channel: string, listener: SimpleListener): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.on(channel, (_, ...args) => listener(...args))
}

/**
 * 添加一次性监听器
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function once(channel: string, listener: IpcRendererListener): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.once(channel, listener)
}

/**
 * 添加一次性监听器（简化版，不包含 event 参数）
 * @param channel 频道名称
 * @param listener 监听器函数
 * @returns 返回一个取消监听的函数
 */
export function onceSimple(channel: string, listener: SimpleListener): () => void {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.once(channel, (_, ...args) => listener(...args))
}

/**
 * 移除指定频道的指定监听器
 * @param channel 频道名称
 * @param listener 要移除的监听器函数
 * @deprecated 推荐使用 on() 方法返回的取消函数
 */
export function removeListener(channel: string, listener: (...args: any[]) => void): void {
  const ipcRenderer = getIpcRenderer()
  // @ts-ignore - 已废弃的方法，但仍需支持
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
export function send(channel: string, ...args: any[]): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.send(channel, ...args)
}

/**
 * 向主进程发送消息并等待响应
 * @param channel 频道名称
 * @param args 要发送的参数
 * @returns Promise，解析为主进程的响应
 */
export function invoke<T = any>(channel: string, ...args: any[]): Promise<T> {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.invoke(channel, ...args)
}

/**
 * 向主进程发送同步消息（不推荐使用）
 * @param channel 频道名称
 * @param args 要发送的参数
 * @returns 主进程的同步响应
 */
export function sendSync<T = any>(channel: string, ...args: any[]): T {
  const ipcRenderer = getIpcRenderer()
  return ipcRenderer.sendSync(channel, ...args)
}

/**
 * 向主进程发送消息并传输 MessagePort
 * @param channel 频道名称
 * @param message 要发送的消息
 * @param transfer 要传输的 MessagePort 数组
 */
export function postMessage(channel: string, message: any, transfer?: MessagePort[]): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.postMessage(channel, message, transfer)
}

/**
 * 向指定的 webContents 发送消息
 * @param webContentsId webContents ID
 * @param channel 频道名称
 * @param args 要发送的参数
 * @deprecated 此方法在 Electron 28 中已被移除
 */
export function sendTo(webContentsId: number, channel: string, ...args: any[]): void {
  const ipcRenderer = getIpcRenderer()
  // @ts-ignore - 已废弃的方法
  ipcRenderer.sendTo(webContentsId, channel, ...args)
}

/**
 * 向宿主页面的 webview 元素发送消息
 * @param channel 频道名称
 * @param args 要发送的参数
 */
export function sendToHost(channel: string, ...args: any[]): void {
  const ipcRenderer = getIpcRenderer()
  ipcRenderer.sendToHost(channel, ...args)
}

// 默认导出所有方法
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
  sendSync,
  postMessage,
  /** @deprecated 此方法在 Electron 28 中已被移除 */
  sendTo,
  sendToHost
}
