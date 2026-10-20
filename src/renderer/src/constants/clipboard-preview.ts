export const PREVIEW_LIMITS = {
  text: 12000,
  richText: 200000,
  structuredText: 20000,
  fileRows: 4,
  tableRows: 8,
  tableColumns: 4
} as const

export const PREVIEW_LABELS = {
  details: '详情',
  close: '关闭',
  plainText: '纯文本',
  formatted: '排版',
  original: '原文',
  json: 'JSON',
  table: '表格',
  caption: '附带文本',
  empty: '暂无可预览内容',
  whitespace: '空白文本',
  richUnavailable: '富文本预览不可用',
  richTooLarge: '富文本较大，已省略排版预览',
  imageUnavailable: '图片无法预览',
  imageLoading: '图片加载中…',
  externalImage: '外部图片',
  previewExcerpt: '预览已截取',
  unknownFile: '未命名文件',
  folder: '文件夹',
  file: '文件',
  justNow: '刚刚',
  formats: '内容格式',
  source: '来源应用',
  image: '图片',
  transparent: '透明背景',
  richText: '富文本',
  text: '文本'
} as const

export const PREVIEW_TYPES: Record<string, { label: string; theme: string }> = {
  text: { label: '文本', theme: 'blue' },
  image: { label: '图片', theme: 'green' },
  html: { label: 'HTML', theme: 'purple' },
  url: { label: '链接', theme: 'orange' },
  files: { label: '文件', theme: 'teal' },
  email: { label: '邮箱', theme: 'red' },
  phone: { label: '电话', theme: 'red' },
  code: { label: '代码', theme: 'purple' },
  rtf: { label: 'RTF', theme: 'purple' },
  color: { label: '颜色', theme: 'green' },
  empty: { label: '其他', theme: 'gray' },
  table: { label: '表格', theme: 'teal' },
  json: { label: 'JSON', theme: 'purple' }
}

export const PREVIEW_ACTIONS = {
  restoreAsText: '粘贴为纯文本',
  restoreOcrText: '复制 OCR 文字',
  parseOcr: '解析 OCR',
  parseQRCode: '解析二维码',
  generateQRCode: '生成二维码',
  openInBrowser: '在浏览器中打开',
  manageTags: '标签管理',
  removeCurrentTag: '移除当前标签',
  delete: '删除'
} as const
