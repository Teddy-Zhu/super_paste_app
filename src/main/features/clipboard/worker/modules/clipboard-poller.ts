import {
  ClipboardManager as TeddyClipboardManager,
  ClipboardListener,
  ClipboardData,
  isWaylandClipboardAvailable
} from '@teddyzhu/clipboard'
import logger from '../../../../utils/worker-logger'
import { EventEmitter } from 'events'
import { isWayland } from '../../../../system/session'

export class ClipboardPoller extends EventEmitter {
  private clipboardInstance: TeddyClipboardManager | null = null
  private accessError: string | undefined

  private get clipboard(): TeddyClipboardManager {
    return (this.clipboardInstance ??= new TeddyClipboardManager())
  }
  private clipboardListener: ClipboardListener
  private isListening: boolean = false

  constructor() {
    super()
    this.clipboardListener = new ClipboardListener()
  }

  public startListening(callback: (data: ClipboardData) => void): void {
    if (this.isListening) return

    try {
      if (isWayland && !isWaylandClipboardAvailable()) {
        throw new Error(
          '当前桌面未提供 Wayland 后台剪贴板访问协议。请启用桌面的剪贴板管理支持，或使用 X11 会话。'
        )
      }
      this.clipboardListener.watch(callback)
      this.isListening = true
      this.accessError = undefined
    } catch (error) {
      this.accessError = error instanceof Error ? error.message : String(error)
      throw error
    }
    logger.debug('[ClipboardPoller] Started listening')
  }

  public stopListening(): void {
    if (!this.isListening) return

    this.clipboardListener.stop()
    this.isListening = false
    logger.debug('[ClipboardPoller] Stopped listening')
  }

  public isListeningActive(): boolean {
    return this.isListening
  }

  public getAccessStatus(): { listening: boolean; available: boolean; error?: string } {
    const available = !isWayland || isWaylandClipboardAvailable()
    return {
      listening: this.isListening,
      available,
      error: this.accessError || (available ? undefined : '桌面未提供 Wayland 后台剪贴板访问协议')
    }
  }

  /**
   * Reads clipboard content directly.
   * Mirrors the original readClipboardDirect logic.
   */
  public readClipboardDirect(): any {
    try {
      // Safe text access
      let text: string | undefined = undefined
      try {
        text = this.clipboard.getText()
      } catch (e) {}

      // Safe HTML access
      let html: string | undefined = undefined
      try {
        html = this.clipboard.getHtml()
      } catch (e) {}

      // Safe RTF access
      let rtf: string | undefined = undefined
      try {
        rtf = this.clipboard.getRichText()
      } catch (e) {}

      // Safe Files access
      let files: string[] | undefined = undefined
      let hasFiles = false
      let filesInfo: { count: number; totalSize: number } | undefined = undefined
      try {
        files = this.clipboard.getFiles()
        hasFiles = !!(files && files.length > 0)
        if (hasFiles) {
          filesInfo = {
            count: files!.length,
            totalSize: files!.reduce((total, file) => total + file.length, 0)
          }
        }
      } catch (e) {}

      // Image access
      let imageData: any = null
      let hasImage = false
      try {
        imageData = this.clipboard.getImageData()
        hasImage = !!(imageData && imageData.data && Buffer.isBuffer(imageData.data))
      } catch (e) {
        hasImage = false
      }

      const result: any = {
        text: text || undefined,
        html: html || undefined,
        rtf: rtf || undefined,
        files: files || undefined,
        hasImage: hasImage,
        hasFiles: hasFiles
      }

      if (hasImage && imageData) {
        result.imageInfo = {
          width: imageData.width || 0,
          height: imageData.height || 0,
          quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
        }
        result.image = imageData.data
      }

      if (hasFiles && filesInfo) {
        result.filesInfo = filesInfo
      }

      return result
    } catch (error) {
      logger.error('[ClipboardPoller] Direct read failed:', error)
      return {}
    }
  }

  public setContents(data: any): void {
    this.clipboard.setContents(data)
  }
}
