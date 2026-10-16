export interface ShortcutStatus {
  accelerator: string
  state: 'disabled' | 'registered' | 'portal-requested' | 'failed'
  error?: string
}

export interface SystemAccessStatus {
  wayland: boolean
  shortcut: ShortcutStatus
  clipboard: { available: boolean; listening: boolean; error?: string }
  sourceTracking: boolean
  executable: string
}
