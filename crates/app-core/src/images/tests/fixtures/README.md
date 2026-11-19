`cmyk-baseline.jpg` 与 `cmyk-progressive.jpg` 是本项目生成的 3×2 JPEG 测试输入，无第三方图片内容。六个像素的常规 CMYK 墨量按行排列为白、黑、青、品红、黄、50% 黑：`(0,0,0,0), (0,0,0,255), (255,0,0,0), (0,255,0,0), (0,0,255,0), (0,0,0,128)`。

生成使用 Pillow 12.3.0 / libjpeg 8.0：创建 `Image.new('CMYK', (3, 2))` 后 `putdata`，分别以 `quality=100, subsampling=0, progressive=False/True` 保存为 JPEG。两个文件均包含 Adobe CMYK APP14 标记，不包含 ICC；测试通过 MoxCMS 构造独立 ICC 并嵌入 APP2，Pillow 不参与测试或应用运行。
