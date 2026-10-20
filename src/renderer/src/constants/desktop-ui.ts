export const SETTINGS_LABELS = {
  navigation: '设置分类',
  preferences: '偏好设置',
  minimize: '最小化',
  close: '关闭',
  editTag: '编辑标签',
  deleteTag: '删除标签',
  unsavedChanges: '有未保存的更改',
  keepTime: '历史记录保存时间',
  overlayPosition: '粘贴面板出现位置'
} as const

export const KEEP_TIME_RANGE = { min: 1, max: 365 } as const

export const OVERLAY_POSITION_OPTIONS = [
  { label: '上方', value: 'top' },
  { label: '下方', value: 'bottom' },
  { label: '左侧', value: 'left' },
  { label: '右侧', value: 'right' }
] as const

export const OVERLAY_LABELS = {
  search: '搜索剪贴板历史',
  searchPlaceholder: '搜索剪贴板…',
  menu: '更多操作',
  empty: '暂无剪贴板记录',
  emptyHint: '复制内容后将在此处显示',
  noResults: '未找到匹配的记录',
  noResultsHint: '尝试其他关键词或切换分类'
} as const
