import { invoke } from './ipc'

import type { SystemAccessStatus } from '../../../shared/system-access'
export type { SystemAccessStatus } from '../../../shared/system-access'

export async function getSystemAccessStatus(retry = false): Promise<SystemAccessStatus> {
  const result = await invoke<{ success: boolean; data?: SystemAccessStatus; error?: string }>(
    retry ? 'system-access-retry' : 'system-access-status'
  )
  if (!result.success || !result.data) throw new Error(result.error || '读取系统访问状态失败')
  return result.data
}

export const SYSTEM_ACCESS_TEXT = {
  title: '系统访问',
  refresh: '刷新状态',
  retry: '重新连接',
  portal: '已向桌面提交快捷键请求，请在系统授权窗口或系统快捷键设置中确认。',
  registered: '快捷键已注册',
  disabled: '尚未设置全局快捷键',
  clipboardReady: '剪贴板监听已启动',
  clipboardStopped: '剪贴板监听未启动',
  wayland:
    'Wayland 的全局快捷键由桌面授权；后台剪贴板监听需要桌面提供相应协议，无法通过普通权限对话框开启。活动窗口来源识别暂不可用。',
  fallback: '若桌面不支持全局快捷键授权，可在系统自定义快捷键中设置以下启动命令：'
} as const
