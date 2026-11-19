pub const MCP_HOST: &str = "127.0.0.1";
pub const MCP_PATH: &str = "/mcp";
pub const MCP_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MCP_IMAGE_PIXEL_LIMIT: u64 = 40_000_000;
pub const MCP_CLIPBOARD_RESOURCE_URI: &str = "super-paste://clipboard/current";
pub const MCP_CLIPBOARD_EVENT_META_KEY: &str = "super-paste/clipboard";
pub const MCP_MAX_BODY_BYTES: usize = 15 * 1024 * 1024;
pub const MCP_MAX_REQUESTS: usize = 32;
pub const MCP_MAX_SESSIONS: usize = 16;
pub const MCP_SESSION_IDLE_SECS: u64 = 5 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpStatus { pub running: bool, pub endpoint: String, pub error: Option<String> }
pub fn mcp_endpoint(port: u16) -> String { format!("http://{MCP_HOST}:{port}{MCP_PATH}") }

/// Shared by the protocol tool catalog and its native settings reference.
#[derive(Clone, Copy)]
pub struct McpToolInfo { pub name: &'static str, pub title: &'static str, pub description: &'static str }
pub const MCP_TOOLS: [McpToolInfo; 8] = [
    McpToolInfo { name: "get_clipboard", title: "读取剪贴板", description: "读取当前系统剪贴板的文本、HTML、RTF、文件路径和图片信息；includeImage=true 时返回 PNG 图片。返回 cursor 可用于等待后续变化。" },
    McpToolInfo { name: "wait_for_clipboard_change", title: "等待剪贴板变化", description: "等待软件监听到剪贴板变化，最多等待 30 秒。首次可省略 cursor，之后传入上次返回的 cursor。返回 changed、cursor、timestamp；变化后调用 get_clipboard 读取当前内容，不保留每次变化的内容快照。" },
    McpToolInfo { name: "subscribe_clipboard_changes", title: "订阅剪贴板变化", description: "在当前 MCP 会话订阅剪贴板变化，通过 Streamable HTTP GET /mcp 的 SSE 连接持续推送 notifications/resources/updated。params._meta[\"super-paste/clipboard\"] 包含 cursor、timestamp 和 content（文本、HTML、RTF、文件路径、图片字节数）。图片数据用 get_clipboard 获取。客户端须保持 SSE 连接；重连后重新读取当前内容，通知不提供离线重放。" },
    McpToolInfo { name: "unsubscribe_clipboard_changes", title: "取消剪贴板订阅", description: "取消当前 MCP 会话的剪贴板变化推送，不影响其他客户端的订阅。" },
    McpToolInfo { name: "search_clipboard_history", title: "搜索剪贴板历史", description: "按内容或 OCR 文字搜索本地剪贴板历史，按复制时间倒序分页返回内容和记录 ID，图片仅返回元数据。" },
    McpToolInfo { name: "get_clipboard_history_item", title: "读取历史记录", description: "按记录 ID 获取一条剪贴板历史，包括文本、OCR 结果、格式和来源；includeImage=true 时返回 PNG 图片。" },
    McpToolInfo { name: "set_clipboard_text", title: "写入剪贴板文本", description: "将指定纯文本写入系统剪贴板，覆盖当前内容；不会模拟粘贴或键盘输入。" },
    McpToolInfo { name: "ocr_image", title: "识别图片文字", description: "使用软件当前 OCR 设置识别图片并直接返回文字。提供 imageBase64（原始 Base64 或图片 data URL）或历史 itemId，均不提供则识别当前剪贴板图片。两者不可同时提供。图片最大 10 MiB，不读取任意文件路径或 URL。本地模式须先在 OCR 设置中下载模型；远程或 AI 模式会使用已配置服务并可能产生费用。" },
];
