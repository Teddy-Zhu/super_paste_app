/**
 * 二维码解析服务
 * 使用 qrcode-reader 和 jimp 解析图片中的二维码
 */

import { Jimp } from "jimp";


import QrCode from 'qrcode-reader';
import logger from '../utils/logger'

/**
 * 二维码服务类
 */
export class QRCodeService {
  private qr: any

  constructor() {
    this.qr = new QrCode()
    logger.debug('[二维码服务] 初始化完成')
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

      // 创建Promise来处理二维码解析的回调
      const result = await new Promise<string | null>((resolve) => {
        this.qr.callback = (err: any, value: any) => {
          if (err) {
            logger.debug('[二维码服务] 解析失败:', err.message)
            resolve(null)
          } else {
            logger.debug('[二维码服务] 解析成功:', value?.result)
            resolve(value?.result || null)
          }
        }

        try {
          // 开始解码
          this.qr.decode(image.bitmap)
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
