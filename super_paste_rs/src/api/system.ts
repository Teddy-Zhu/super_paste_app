import { invoke } from '@tauri-apps/api/core';

export const systemApi = {
  /**
   * 显示 Overlay 窗口（根据配置的位置）
   */
  async showOverlay(): Promise<void> {
    return invoke('show_overlay_window');
  },

  /**
   * 隐藏 Overlay 窗口
   */
  async hideOverlay(): Promise<void> {
    return invoke('hide_overlay_window');
  },

  /**
   * 切换 Overlay 窗口显示/隐藏
   */
  async toggleOverlay(): Promise<void> {
    return invoke('toggle_overlay_window');
  },
};
