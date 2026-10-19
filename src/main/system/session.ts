export const isWayland =
  process.platform === 'linux' &&
  (process.env.XDG_SESSION_TYPE === 'wayland' || Boolean(process.env.WAYLAND_DISPLAY))

// 面板的绝对定位和置顶依赖 X11，Wayland 桌面通过 XWayland 提供这些能力。
export const useXWayland = isWayland && Boolean(process.env.DISPLAY)
