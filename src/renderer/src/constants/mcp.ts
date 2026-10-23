import {
  getMCPEndpoint,
  MCP_TEXT,
  MCP_CLIPBOARD_RESOURCE_URI,
  MCP_CLIPBOARD_EVENT_META_KEY,
  type MCPSettings
} from '@shared/mcp'

export const MCP_PUSH_EXAMPLE = JSON.stringify(
  {
    jsonrpc: '2.0',
    method: 'notifications/resources/updated',
    params: {
      uri: MCP_CLIPBOARD_RESOURCE_URI,
      _meta: {
        [MCP_CLIPBOARD_EVENT_META_KEY]: {
          cursor: '<cursor>',
          timestamp: 1760000000000,
          content: { text: '示例内容' }
        }
      }
    }
  },
  null,
  2
)

export const MCP_CLIENTS = [
  { id: 'cursor', name: 'Cursor', help: MCP_TEXT.cursorHelp, docs: 'https://cursor.com/docs/mcp' },
  {
    id: 'claude',
    name: 'Claude Code',
    help: MCP_TEXT.claudeHelp,
    docs: 'https://code.claude.com/docs/en/mcp'
  },
  {
    id: 'codex',
    name: 'Codex',
    help: MCP_TEXT.codexHelp,
    docs: 'https://developers.openai.com/codex/mcp'
  },
  {
    id: 'vscode',
    name: 'VS Code / Copilot',
    help: MCP_TEXT.vscodeHelp,
    docs: 'https://code.visualstudio.com/docs/agents/reference/mcp-configuration'
  }
] as const

export type MCPClientId = (typeof MCP_CLIENTS)[number]['id']

export function createMCPClientConfig(
  client: MCPClientId,
  settings: MCPSettings,
  revealKey = false
): string {
  const url = getMCPEndpoint(settings.port)
  if (client === 'codex') {
    return `[mcp_servers.super-paste]\nurl = ${JSON.stringify(url)}\n${settings.authEnabled ? 'bearer_token_env_var = "SUPER_PASTE_MCP_KEY"\n' : ''}tool_timeout_sec = 180`
  }
  const key = revealKey && settings.apiKey ? settings.apiKey : MCP_TEXT.keyPlaceholder
  const server = {
    ...(client === 'cursor' ? {} : { type: 'http' }),
    url,
    ...(settings.authEnabled ? { headers: { Authorization: `Bearer ${key}` } } : {})
  }
  return JSON.stringify(
    { [client === 'vscode' ? 'servers' : 'mcpServers']: { 'super-paste': server } },
    null,
    2
  )
}
