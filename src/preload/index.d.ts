import type { IpcRendererEvent } from 'electron'

export type IpcRendererListener<Args extends unknown[] = unknown[]> = (
  event: IpcRendererEvent,
  ...args: Args
) => void

/** 渲染进程可用的受控 Electron 桥接接口。 */
export interface CustomElectronAPI {
  ipcRenderer: {
    on<Args extends unknown[]>(channel: string, listener: IpcRendererListener<Args>): () => void
    once<Args extends unknown[]>(channel: string, listener: IpcRendererListener<Args>): () => void
    send(channel: string, ...args: unknown[]): void
    invoke(channel: string, ...args: unknown[]): Promise<unknown>
    sendSync(channel: string, ...args: unknown[]): unknown
    off<Args extends unknown[]>(channel: string, listener: IpcRendererListener<Args>): void
    removeListener<Args extends unknown[]>(
      channel: string,
      listener: IpcRendererListener<Args>
    ): void
    removeAllListeners(channel: string): void
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
