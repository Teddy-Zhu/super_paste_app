export const OVERLAY_RENDERER_EVENTS = [
  'prepare',
  'prepared',
  'opening',
  'opened',
  'closing',
  'closed',
  'hidden',
  'cancelled',
  'ignored',
  'error'
] as const

export interface OverlayRendererTrace {
  event: (typeof OVERLAY_RENDERER_EVENTS)[number]
  transitionId: number
  animationVersion: number
  animationState: string
  sentAt: number
  overlayVisible: boolean
  documentHidden: boolean
  documentFocused: boolean
  elapsedMs?: number
  receivedId?: number
  reason?: string
  transform?: string
  viewportWidth?: number
  viewportHeight?: number
  devicePixelRatio?: number
}
