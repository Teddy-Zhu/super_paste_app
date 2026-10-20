import type { ClipboardData } from '@teddyzhu/clipboard'
import {
  ClipboardContent,
  ClipboardSnapshot,
  ClipboardDataConverter
} from '../../types/clipboard-types'
import { HashUtils } from '../../../../utils/common'
import logger from '../../../../utils/worker-logger'

export class ContentProcessor {
  private lastQuickHash: string = ''

  /**
   * Convert raw ClipboardData to internal structure
   */
  public convertClipboardData(data: ClipboardData): ClipboardSnapshot {
    const result: ClipboardSnapshot = {
      text: data.text || undefined,
      html: data.html || undefined,
      rtf: data.rtf || undefined,
      hasImage: !!data.image,
      hasFiles: !!(data.files && data.files.length > 0)
    }

    if (data.files && data.files.length > 0) {
      result.files = data.files
      result.filesInfo = {
        count: data.files.length,
        totalSize: data.files.reduce((total, file) => total + file.length, 0)
      }
    }

    if (data.image) {
      const imageData = data.image
      result.imageInfo = {
        width: imageData.width || 0,
        height: imageData.height || 0,
        quickId: `${imageData.width || 0}x${imageData.height || 0}_${imageData.size || 0}`
      }
      result.image = imageData.data
    }

    return result
  }

  /**
   * Check for quick changes to avoid full processing
   */
  public checkQuickClipboardChange(clipboardData: ClipboardSnapshot): boolean {
    try {
      const quickContent = {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        hasImage: clipboardData.hasImage,
        hasFiles: clipboardData.hasFiles,
        imageInfo: clipboardData.imageInfo || undefined,
        filesInfo: clipboardData.filesInfo || undefined
      }

      const quickHash = HashUtils.generateQuickHash(quickContent)

      if (!this.lastQuickHash) {
        this.lastQuickHash = quickHash
        return true
      }

      const hasChange = quickHash !== this.lastQuickHash
      if (hasChange) {
        this.lastQuickHash = quickHash
      }

      return hasChange
    } catch (error) {
      logger.warn('[ContentProcessor] 无法比较剪贴板快照，将重新处理:', error)
      return true
    }
  }

  public async generateClipboardHash(clipboardData: ClipboardSnapshot): Promise<string> {
    try {
      const content = await this.buildClipboardContent(clipboardData)
      return ClipboardDataConverter.generateContentHash(content)
    } catch (error) {
      logger.error('[ContentProcessor] 生成剪贴板哈希失败:', error)
      throw error
    }
  }

  public async buildClipboardContent(clipboardData: ClipboardSnapshot): Promise<ClipboardContent> {
    try {
      if (clipboardData.hasImage && clipboardData.image) {
        return {
          text: clipboardData.text || undefined,
          html: clipboardData.html || undefined,
          rtf: clipboardData.rtf || undefined,
          files: clipboardData.files || undefined,
          image: clipboardData.image
        }
      }

      return {
        text: clipboardData.text || undefined,
        html: clipboardData.html || undefined,
        rtf: clipboardData.rtf || undefined,
        files: clipboardData.files || undefined
      }
    } catch (error) {
      logger.error('[ContentProcessor] 构建剪贴板内容失败:', error)
      throw error
    }
  }

  /**
   * Convert ClipboardContent to library format for restoration
   */
  public prepareForRestore(content: ClipboardContent): ClipboardData {
    const data: ClipboardData = {
      availableFormats: [] as string[]
    }

    if (content.text) {
      data.text = content.text
      data.availableFormats.push('text')
    }

    if (content.html) {
      data.html = content.html
      data.availableFormats.push('html')
    }

    if (content.rtf) {
      data.rtf = content.rtf
      data.availableFormats.push('rtf')
    }

    if (content.image) {
      // Image is Buffer
      data.image = {
        width: 0,
        height: 0,
        size: content.image.length,
        data: content.image
      }
      data.availableFormats.push('image')
    }

    if (content.files && content.files.length > 0) {
      data.files = content.files
      data.availableFormats.push('files')
    }

    return data
  }

  /**
   * Convert various formats to text
   */
  public convertToText(content: ClipboardContent): string {
    if (content.text && content.text.trim()) {
      return content.text
    }

    // HTML to Text
    if (content.html && content.html.trim()) {
      try {
        const textFromHtml = content.html
          .replace(/<script[^>]*>[\s\S]*?<\/script>/gi, '')
          .replace(/<style[^>]*>[\s\S]*?<\/style>/gi, '')
          .replace(/<[^>]*>/g, '')
          .replace(/&nbsp;/g, ' ')
          .replace(/&lt;/g, '<')
          .replace(/&gt;/g, '>')
          .replace(/&amp;/g, '&')
          .replace(/&quot;/g, '"')
          .replace(/&#39;/g, "'")
          .replace(/\s+/g, ' ')
          .trim()

        if (textFromHtml) return textFromHtml
      } catch (error) {
        logger.warn('[ContentProcessor] 转换纯文本失败，继续尝试其他格式:', error)
      }
    }

    // RTF to Text
    if (content.rtf && content.rtf.trim()) {
      try {
        const textFromRtf = content.rtf
          .replace(/\\[a-z]+\d*\s?/g, '')
          .replace(/[{}]/g, '')
          .replace(/\\\\/g, '\\')
          .replace(/\\'/g, "'")
          .replace(/\s+/g, ' ')
          .trim()

        if (textFromRtf) return textFromRtf
      } catch (error) {
        logger.warn('[ContentProcessor] 转换纯文本失败，继续尝试其他格式:', error)
      }
    }

    // Files to Text list
    if (content.files && content.files.length > 0) {
      return content.files.join('\n')
    }

    return ''
  }
}
