import type { ActiveWindowInfo } from '../../../../services/active-window-provider'

function isScreenshotTool(window: ActiveWindowInfo | null): boolean {
  return window?.application.toLowerCase() === 'mark-shot'
}

export class ClipboardSourceTracker {
  private currentWindow: ActiveWindowInfo | null = null
  private screenshotSource: ActiveWindowInfo | null = null

  get activeWindow(): ActiveWindowInfo | null {
    return this.currentWindow
  }

  update(window: ActiveWindowInfo | null): void {
    if (isScreenshotTool(window)) {
      // Keep the target while the capture overlay changes its title or window.
      if (!isScreenshotTool(this.currentWindow)) this.screenshotSource = this.currentWindow
    } else {
      this.screenshotSource = null
    }
    this.currentWindow = window
  }

  select(hasImage: boolean): {
    window: ActiveWindowInfo | null
    reason: 'active-window' | 'before-screenshot' | 'screenshot-target-unavailable'
  } {
    if (hasImage && isScreenshotTool(this.currentWindow)) {
      if (this.screenshotSource?.application && this.screenshotSource.path) {
        return { window: this.screenshotSource, reason: 'before-screenshot' }
      }
      return { window: this.currentWindow, reason: 'screenshot-target-unavailable' }
    }
    return { window: this.currentWindow, reason: 'active-window' }
  }
}
