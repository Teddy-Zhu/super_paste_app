export const MCP_HOST = '127.0.0.1'
export const MCP_PATH = '/mcp'
export const MCP_MAX_IMAGE_BYTES = 10 * 1024 * 1024
export const MCP_CLIPBOARD_RESOURCE_URI = 'super-paste://clipboard/current'
export const MCP_CLIPBOARD_EVENT_META_KEY = 'super-paste/clipboard'

export interface ClipboardChangeData {
  timestamp: number
  content: {
    text?: string
    html?: string
    rtf?: string
    files?: string[]
    ocr?: string
    image?: { bytes: number }
  }
}

export interface MCPClipboardChange extends ClipboardChangeData {
  cursor: string
}

export interface MCPSettings {
  enabled: boolean
  port: number
  authEnabled: boolean
  apiKey: string
}

export interface MCPStatus {
  running: boolean
  endpoint: string
  error?: string
}

export interface MCPState {
  settings: MCPSettings
  status: MCPStatus
}

export const createDefaultMCPSettings = (): MCPSettings => ({
  enabled: false,
  port: 19527,
  authEnabled: true,
  apiKey: ''
})

export const getMCPEndpoint = (port: number): string => `http://${MCP_HOST}:${port}${MCP_PATH}`

export const MCP_TOOLS = {
  get_clipboard: {
    title: '读取剪贴板',
    description:
      '读取当前系统剪贴板的文本、HTML、RTF、文件路径和图片信息；includeImage=true 时返回 PNG 图片。返回 cursor 可用于等待后续变化。'
  },
  wait_for_clipboard_change: {
    title: '等待剪贴板变化',
    description:
      '等待软件监听到剪贴板变化，最多等待 30 秒。首次可省略 cursor，之后传入上次返回的 cursor。返回 changed、cursor、timestamp；变化后调用 get_clipboard 读取当前内容，不保留每次变化的内容快照。'
  },
  subscribe_clipboard_changes: {
    title: '订阅剪贴板变化',
    description:
      '在当前 MCP 会话订阅剪贴板变化，通过 Streamable HTTP GET /mcp 的 SSE 连接持续推送 notifications/resources/updated。params._meta["super-paste/clipboard"] 包含 cursor、timestamp 和 content（文本、HTML、RTF、文件路径、图片字节数）。图片数据用 get_clipboard 获取。客户端须保持 SSE 连接；重连后重新读取当前内容，通知不提供离线重放。'
  },
  unsubscribe_clipboard_changes: {
    title: '取消剪贴板订阅',
    description: '取消当前 MCP 会话的剪贴板变化推送，不影响其他客户端的订阅。'
  },
  search_clipboard_history: {
    title: '搜索剪贴板历史',
    description:
      '按内容或 OCR 文字搜索本地剪贴板历史，按复制时间倒序分页返回内容和记录 ID，图片仅返回元数据。'
  },
  get_clipboard_history_item: {
    title: '读取历史记录',
    description:
      '按记录 ID 获取一条剪贴板历史，包括文本、OCR 结果、格式和来源；includeImage=true 时返回 PNG 图片。'
  },
  set_clipboard_text: {
    title: '写入剪贴板文本',
    description: '将指定纯文本写入系统剪贴板，覆盖当前内容；不会模拟粘贴或键盘输入。'
  },
  ocr_image: {
    title: '识别图片文字',
    description:
      '使用软件当前 OCR 设置识别图片并直接返回文字。提供 imageBase64（原始 Base64 或图片 data URL）或历史 itemId，均不提供则识别当前剪贴板图片。两者不可同时提供。图片最大 10 MiB，不读取任意文件路径或 URL。本地模式须先在 OCR 设置中下载模型；远程或 AI 模式会使用已配置服务并可能产生费用。'
  }
} as const

export const MCP_TEXT = {
  title: 'MCP 服务',
  subtitle: '通过本地 HTTP 服务向 AI 工具开放剪贴板与 OCR 能力。',
  enabled: '开启 MCP',
  enabledDescription: '保存后生效，软件运行期间提供服务，下次启动自动恢复。',
  port: '服务端口',
  portDescription: '仅监听 127.0.0.1，端口范围 1024–65535。',
  auth: 'API Key 验证',
  authDescription: '开启后，每个请求须携带 Authorization: Bearer <API_KEY>。',
  key: 'API Key',
  keyDescription: '8–256 位可见 ASCII 字符，不含空格；密钥仅保存在本机。',
  generate: '生成密钥',
  reveal: '显示密钥',
  hide: '隐藏密钥',
  save: '保存并应用',
  saving: '正在应用…',
  saved: '配置已保存',
  loading: '正在加载…',
  running: '运行中',
  stopped: '未运行',
  refresh: '刷新状态',
  address: '服务地址',
  copy: '复制',
  copyKey: '复制密钥',
  copyConfig: '复制配置',
  copied: '已复制',
  copyFailed: '复制失败，请手动复制',
  capabilities: '可用工具',
  clients: 'AI 工具配置',
  clientDescription: '示例使用已保存配置。保存修改后复制到对应工具，并重新连接 MCP 服务。',
  configSecretNotice: '复制的配置包含已保存的 API Key，请保存在本机私有配置中。',
  keyPlaceholder: 'YOUR_API_KEY',
  localNotice: 'AI 工具须与 Super Paste 运行在同一台电脑。云端连接器无法直接访问此本地地址。',
  accessNotice:
    '开启后，连接的工具可读取剪贴板和历史记录、写入文本。关闭密钥验证时，本机其他程序也可访问。',
  ocrNotice: 'OCR 使用当前 OCR 设置；本地模型须先下载，远程与 AI 模式可能产生网络请求和费用。',
  changesNotice: '支持 Streamable HTTP / SSE 实时推送；订阅与等待调用均依赖软件的剪贴板监听权限。',
  pushTitle: '实时订阅',
  pushDescription:
    '调用 subscribe_clipboard_changes，或通过 resources/subscribe 订阅 super-paste://clipboard/current。剪贴板变化时，SSE 推送 notifications/resources/updated，消息的 _meta 中附带变化时的内容快照。图片仅推送字节数。',
  pushConnection:
    '使用初始化响应中的 Mcp-Session-Id，在同一 /mcp 地址建立 GET 连接（Accept: text/event-stream）；每次 GET、POST、DELETE 均须携带会话 ID 和已配置的密钥。标准 MCP 客户端可自动管理连接。',
  pushCompatibility:
    'AI 工具对资源通知的处理方式不同，接收推送不代表会自动触发模型回复。不支持订阅时可调用 wait_for_clipboard_change。SSE 重连后请重新读取当前内容；断线通知不重放，离线会话将在 5 分钟后清理。',
  pushCancel:
    '调用 unsubscribe_clipboard_changes、resources/unsubscribe 或 DELETE /mcp 可结束订阅。',
  docs: '官方文档',
  cursorHelp:
    '将配置合并到 ~/.cursor/mcp.json（全局）或项目的 .cursor/mcp.json，在 MCP 设置中启用 Super Paste。',
  claudeHelp:
    '将配置合并到项目 .mcp.json，重新打开 Claude Code，按提示启用项目 MCP 服务，通过 /mcp 查看连接状态。',
  codexHelp:
    '将以下内容合并到 ~/.codex/config.toml，然后重新打开 Codex。启用验证时，在启动 Codex 的环境中设置 SUPER_PASTE_MCP_KEY 为本页密钥。',
  vscodeHelp:
    '将配置合并到项目 .vscode/mcp.json（Local 配置格式），通过 MCP: List Servers 启动服务，并在聊天工具列表中启用。',
  promptTitle: '调用示例',
  prompt:
    '使用 Super Paste MCP 读取当前剪贴板；如果是图片，调用 ocr_image 识别文字。需要接收后续复制内容时，调用 subscribe_clipboard_changes 订阅实时推送。'
} as const
