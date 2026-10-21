import { createJimp } from '@jimp/core'
import bmp, { msBmp } from '@jimp/js-bmp'
import gif from '@jimp/js-gif'
import jpeg from '@jimp/js-jpeg'
import png from '@jimp/js-png'
import tiff from '@jimp/js-tiff'
import { methods } from '@jimp/plugin-color'

// 保留原有解码格式，仅装配二维码解析和 AI OCR 所需的灰度处理。
export const Jimp = createJimp({
  formats: [bmp, msBmp, gif, jpeg, png, tiff],
  plugins: [{ greyscale: methods.greyscale }]
})
