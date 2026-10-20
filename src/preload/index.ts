import { contextBridge, ipcRenderer } from 'electron'
import type { CustomElectronAPI } from './index.d'

/**
 * 自定义 Electron API
 * 只暴露项目中实际使用的 API，提高安全性
 */
const electronAPI: CustomElectronAPI = {
  // IPC 渲染进程 API
  ipcRenderer: {
    // 监听消息
    on: (channel: string, listener) => {
      ipcRenderer.on(channel, listener)
      // 返回取消监听的函数
      return () => {
        ipcRenderer.off(channel, listener)
      }
    },

    // 一次性监听
    once: (channel: string, listener) => {
      ipcRenderer.once(channel, listener)
      // 返回取消监听的函数
      return () => {
        ipcRenderer.off(channel, listener)
      }
    },

    // 发送异步消息
    send: (channel: string, ...args: unknown[]) => {
      ipcRenderer.send(channel, ...args)
    },

    // 发送消息并等待响应
    invoke: (channel: string, ...args: unknown[]) => {
      return ipcRenderer.invoke(channel, ...args)
    },

    // 发送同步消息（仅在必要时使用）
    sendSync: (channel: string, ...args: unknown[]) => {
      return ipcRenderer.sendSync(channel, ...args)
    },

    // 移除监听器
    off: (channel: string, listener) => {
      ipcRenderer.off(channel, listener)
    },

    // 移除监听器（已废弃，但项目中仍在使用）
    removeListener: (channel: string, listener) => {
      ipcRenderer.removeListener(channel, listener)
    },

    // 移除所有监听器
    removeAllListeners: (channel: string) => {
      ipcRenderer.removeAllListeners(channel)
    }
  },

  // 进程信息 API
  process: {
    platform: process.platform,
    versions: process.versions
  }
}

// 使用 contextBridge 安全地暴露 API
if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld('electron', electronAPI)
  } catch (error) {
    console.error('Failed to expose electron API:', error)
  }
} else {
  window.electron = electronAPI
}
