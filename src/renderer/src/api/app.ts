import { invoke } from './ipc'

export const app = {
  async restart(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:restart')
      return result.success
    } catch (error) {
      console.error('重启应用失败:', error)
      return false
    }
  },

  async minimizeWindow(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:window-minimize')
      return result.success
    } catch (error) {
      console.error('最小化窗口失败:', error)
      return false
    }
  },

  async closeWindow(): Promise<boolean> {
    try {
      const result: { success: boolean; error?: string } = await invoke('app:window-close')
      return result.success
    } catch (error) {
      console.error('关闭窗口失败:', error)
      return false
    }
  }
}
