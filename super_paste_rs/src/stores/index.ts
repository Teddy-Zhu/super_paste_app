import { createPinia } from 'pinia';

const store = createPinia();

// TODO: 实现 Tauri 版本的持久化和字段变更监听
// Tauri 不支持同步 IPC，需要使用异步方式

export default store;

// 导出所有 stores
export * from './common';

