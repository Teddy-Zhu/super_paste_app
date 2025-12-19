import {
    ClipboardManager as TeddyClipboardManager,
    ClipboardListener,
    ClipboardData
} from '@teddyzhu/clipboard'
import logger from '../../../../utils/logger'
import { EventEmitter } from 'events'

export class ClipboardPoller extends EventEmitter {
    private clipboard: TeddyClipboardManager
    private clipboardListener: ClipboardListener
    private isListening: boolean = false

    constructor() {
        super()
        this.clipboard = new TeddyClipboardManager()
        this.clipboardListener = new ClipboardListener()
    }

    public startListening(callback: (data: ClipboardData) => void): void {
        if (this.isListening) return

        this.isListening = true
        this.clipboardListener.watch(callback)
        logger.debug('[ClipboardPoller] Started listening')
    }

    public stopListening(): void {
        if (!this.isListening) return

        this.isListening = false
        this.clipboardListener.stop()
        logger.debug('[ClipboardPoller] Stopped listening')
    }

    public isListeningActive(): boolean {
        return this.isListening
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
            } catch (e) { }

            // Safe HTML access
            let html: string | undefined = undefined
            try {
                html = this.clipboard.getHtml()
            } catch (e) { }

            // Safe RTF access
            let rtf: string | undefined = undefined
            try {
                rtf = this.clipboard.getRichText()
            } catch (e) { }

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
            } catch (e) { }

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
