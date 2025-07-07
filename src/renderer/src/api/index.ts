/**
 * API 模块统一导出
 */

// 导出基础 IPC 方法
export * from './ipc'

// 导出类型定义
export * from './types'

// 导出高级 API
export * from './renderer-api'

// 默认导出
export { default as ipc } from './ipc'
export { default as rendererAPI } from './renderer-api'
