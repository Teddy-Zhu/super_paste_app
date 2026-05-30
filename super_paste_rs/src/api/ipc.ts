/**
 * IPC 通信兼容层 - Tauri 版本
 */

import { listen, once as listenOnce, emit } from '@tauri-apps/api/event';
import { invoke as tauriInvoke } from '@tauri-apps/api/core';

export type SimpleListener = (...args: any[]) => void;

/**
 * 监听事件（带 event 对象）
 */
export function on(channel: string, callback: (event: any, ...args: any[]) => void): () => void {
  let unlisten: (() => void) | null = null;

  listen(channel, (event) => {
    callback(event, event.payload);
  }).then((fn) => {
    unlisten = fn;
  });

  return () => {
    if (unlisten) unlisten();
  };
}

/**
 * 监听事件（仅 payload）
 */
export function onSimple(channel: string, callback: SimpleListener): () => void {
  let unlisten: (() => void) | null = null;

  listen(channel, (event) => {
    callback(event.payload);
  }).then((fn) => {
    unlisten = fn;
  });

  return () => {
    if (unlisten) unlisten();
  };
}

/**
 * 监听一次事件（带 event 对象）
 */
export function once(channel: string, callback: (event: any, ...args: any[]) => void): () => void {
  let unlisten: (() => void) | null = null;

  listenOnce(channel, (event) => {
    callback(event, event.payload);
  }).then((fn) => {
    unlisten = fn;
  });

  return () => {
    if (unlisten) unlisten();
  };
}

/**
 * 监听一次事件（仅 payload）
 */
export function onceSimple(channel: string, callback: SimpleListener): () => void {
  let unlisten: (() => void) | null = null;

  listenOnce(channel, (event) => {
    callback(event.payload);
  }).then((fn) => {
    unlisten = fn;
  });

  return () => {
    if (unlisten) unlisten();
  };
}

/**
 * 发送事件
 */
export function send(channel: string, data?: any): void {
  emit(channel, data);
}

/**
 * 调用后端命令
 */
export async function invoke<T = any>(command: string, args?: any): Promise<T> {
  return tauriInvoke(command, args);
}

/**
 * 同步调用（Tauri 不支持，转为异步）
 */
export function sendSync<T = any>(command: string, args?: any): T {
  throw new Error('Tauri does not support synchronous IPC. Use invoke() instead.');
}

/**
 * 移除监听器（占位）
 */
export function removeListener(channel: string, callback: (...args: any[]) => void): void {
  console.warn('removeListener is not fully implemented in Tauri adapter');
}

/**
 * 移除所有监听器（占位）
 */
export function removeAllListeners(channel: string): void {
  console.warn('removeAllListeners is not fully implemented in Tauri adapter');
}

export default {
  on,
  onSimple,
  once,
  onceSimple,
  send,
  invoke,
  sendSync,
  removeListener,
  removeAllListeners,
};
