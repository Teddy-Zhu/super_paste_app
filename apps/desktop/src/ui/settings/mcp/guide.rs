use super_paste_core::{Error, Result};
use super_paste_format::{McpSettings, mcp_endpoint};
use super::copy;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Client { Cursor, Claude, Codex, VsCode }
impl Client {
    pub const ALL: [Self; 4] = [Self::Cursor, Self::Claude, Self::Codex, Self::VsCode];
    pub fn index(self) -> usize { Self::ALL.iter().position(|client| *client == self).expect("known MCP client") }
    pub fn name(self) -> &'static str { match self { Self::Cursor => "Cursor", Self::Claude => "Claude Code", Self::Codex => "Codex", Self::VsCode => "VS Code / Copilot" } }
    pub fn help(self) -> &'static str { match self { Self::Cursor => copy::CURSOR_HELP, Self::Claude => copy::CLAUDE_HELP, Self::Codex => copy::CODEX_HELP, Self::VsCode => copy::VSCODE_HELP } }
    pub fn docs(self) -> &'static str { match self { Self::Cursor => "https://cursor.com/docs/mcp", Self::Claude => "https://code.claude.com/docs/en/mcp", Self::Codex => "https://developers.openai.com/codex/mcp", Self::VsCode => "https://code.visualstudio.com/docs/agents/reference/mcp-configuration" } }
    pub fn includes_key(self, settings: &McpSettings) -> bool { settings.auth_enabled && self != Self::Codex }
}
/// Client configuration is a deliberate protocol/export boundary. It never
/// becomes an application setting and uses this application's own server alias.
pub fn configuration(client: Client, settings: &McpSettings, key: Option<&str>) -> Result<String> {
    let url = mcp_endpoint(settings.port);
    if client == Client::Codex {
        return Ok(format!("[mcp_servers.super-paste]\nurl = {url:?}\n{}tool_timeout_sec = 180", if settings.auth_enabled { "bearer_token_env_var = \"SUPER_PASTE_MCP_KEY\"\n" } else { "" }));
    }
    let mut server = serde_json::Map::new();
    if client != Client::Cursor { server.insert("type".into(), "http".into()); }
    server.insert("url".into(), url.into());
    if settings.auth_enabled { server.insert("headers".into(), serde_json::json!({ "Authorization": format!("Bearer {}", key.unwrap_or(copy::KEY_PLACEHOLDER)) })); }
    let group = if client == Client::VsCode { "servers" } else { "mcpServers" };
    serde_json::to_string_pretty(&serde_json::json!({ (group): { "super-paste": server } })).map_err(|error| Error::Task(error.to_string().into()))
}
pub fn push_example() -> String {
    serde_json::to_string_pretty(&serde_json::json!({"jsonrpc":"2.0", "method":"notifications/resources/updated", "params": {
        "uri": super_paste_format::MCP_CLIPBOARD_RESOURCE_URI,
        "_meta": { (super_paste_format::MCP_CLIPBOARD_EVENT_META_KEY): { "cursor":"<cursor>", "timestamp":1760000000000_i64, "content":{"text":"示例内容"} } }
    }})).expect("static MCP notification example")
}
