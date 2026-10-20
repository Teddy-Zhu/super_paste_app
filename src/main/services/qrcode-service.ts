/**
 * 二维码解析和生成服务
 * 使用 qrcode-reader 和 jimp 解析图片中的二维码
 * 使用 qrcode 生成二维码
 */

import { Jimp } from 'jimp'
import QRCode from 'qrcode'
import QrCode from 'qrcode-reader'
import logger from '../utils/logger'

/**
 * 二维码服务类
 */
export class QRCodeService {
  constructor() {
    logger.debug('[二维码服务] 初始化完成')
  }

  /**
   * 将文本转换为二维码并返回base64编码的PNG图片
   * @param text 要转换为二维码的文本
   * @param options QRCode选项
   * @returns Promise<string> base64编码的PNG图片数据
   */
  async generateQRCode(text: string, options?: QRCode.QRCodeToDataURLOptions): Promise<string> {
    try {
      if (!text || text.trim() === '') {
        throw new Error('文本内容不能为空')
      }

      // 设置默认选项
      const defaultOptions: QRCode.QRCodeToDataURLOptions = {
        type: 'image/png',
        width: 256,
        margin: 2,
        color: {
          dark: '#000000',
          light: '#FFFFFF'
        },
        errorCorrectionLevel: 'M'
      }

      const finalOptions = { ...defaultOptions, ...options }

      // 生成二维码
      const dataUrl = await QRCode.toDataURL(text, finalOptions)

      logger.info('[二维码服务] 二维码生成成功', { textLength: text.length })
      return dataUrl
    } catch (error) {
      logger.error('[二维码服务] 生成二维码失败:', error)
      throw new Error(`生成二维码失败: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  /**
   * 将文本转换为二维码并返回Buffer
   * @param text 要转换为二维码的文本
   * @param options QRCode选项
   * @returns Promise<Buffer> PNG图片Buffer
   */
  async generateQRCodeBuffer(
    text: string,
    options?: QRCode.QRCodeToBufferOptions
  ): Promise<Buffer> {
    try {
      if (!text || text.trim() === '') {
        throw new Error('文本内容不能为空')
      }

      // 设置默认选项
      const defaultOptions: QRCode.QRCodeToBufferOptions = {
        type: 'png',
        width: 256,
        margin: 2,
        color: {
          dark: '#000000',
          light: '#FFFFFF'
        },
        errorCorrectionLevel: 'M'
      }

      const finalOptions = { ...defaultOptions, ...options }

      // 生成二维码Buffer
      const buffer = await QRCode.toBuffer(text, finalOptions)

      logger.info('[二维码服务] 二维码Buffer生成成功', {
        textLength: text.length,
        bufferSize: buffer.length
      })
      return buffer
    } catch (error) {
      logger.error('[二维码服务] 生成二维码Buffer失败:', error)
      throw new Error(
        `生成二维码Buffer失败: ${error instanceof Error ? error.message : String(error)}`
      )
    }
  }

  /**
   * 解析图片数据中的二维码
   * @param imageBuffer 图片数据buffer
   * @returns Promise<string | null> 解析到的文本内容或null
   */
  async parseFromBuffer(imageBuffer: Buffer): Promise<string | null> {
    try {
      // 使用 Jimp 加载图片
      const image = await Jimp.read(imageBuffer)

      // 转换为灰度图以提高识别率
      image.greyscale()

      // 每次解析使用独立实例，避免并发任务覆盖回调。
      const reader: {
        callback: (error: unknown, value?: { result?: string }) => void
        decode(bitmap: typeof image.bitmap): void
      } = new QrCode()
      const result = await new Promise<string | null>((resolve) => {
        reader.callback = (err, value) => {
          if (err) {
            logger.debug('[二维码服务] 解析失败:', err)
            resolve(null)
          } else {
            logger.debug('[二维码服务] 解析成功:', value?.result)
            resolve(value?.result || null)
          }
        }

        try {
          // 开始解码
          reader.decode(image.bitmap)
        } catch (error) {
          logger.debug('[二维码服务] 解码过程出错:', error)
          resolve(null)
        }
      })

      return result
    } catch (error) {
      logger.error('[二维码服务] 处理图片失败:', error)
      return null
    }
  }
}

// 导出单例实例
export const qrCodeService = new QRCodeService()
export default qrCodeService
