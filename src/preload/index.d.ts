/**
 * 自定义 Electron API 类型定义
 * 只包含项目中实际使用的 API
 */
interface CustomElectronAPI {
  ipcRenderer: {
    on: (channel: string, listener: (...args: any[]) => void) => () => void
    once: (channel: string, listener: (...args: any[]) => void) => () => void
    send: (channel: string, ...args: any[]) => void
    invoke: (channel: string, ...args: any[]) => Promise<any>
    sendSync: (channel: string, ...args: any[]) => any
    removeListener: (channel: string, listener: (...args: any[]) => void) => void
    removeAllListeners: (channel: string) => void
  }
  process: {
    platform: NodeJS.Platform
    versions: NodeJS.ProcessVersions
  }
}

declare global {
  interface Window {
    electron: CustomElectronAPI
  }
}
