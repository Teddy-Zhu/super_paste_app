export interface WindowBounds {
  x: number
  y: number
  width: number
  height: number
}

export interface WindowInfo {
  /** KWin internal UUID, not an X11 window id. */
  id: string
  title: string
  application: string
  windowClass: string
  pid: number
  /** Absent when /proc/<pid>/exe is inaccessible. */
  path?: string | null
  bounds: WindowBounds
}

export interface SupportStatus {
  supported: boolean
  reason?: string
}

/** Checks the session, not D-Bus connectivity or KWin scripting permissions. */
export function getSupportStatus(): SupportStatus

/** A snapshot from one KWin query. An empty desktop returns null. */
export function getActiveWindow(): Promise<WindowInfo | null>

/** Serial polling, deduplicated snapshots, and an explicit error callback. */
export function subscribe(
  onChange: (window: WindowInfo | null) => void,
  onError: (error: Error) => void,
  intervalMs?: number
): () => void
