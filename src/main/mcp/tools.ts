import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import {
  SubscribeRequestSchema,
  UnsubscribeRequestSchema,
  McpError,
  ErrorCode,
  type CallToolResult
} from '@modelcontextprotocol/sdk/types.js'
import { z } from 'zod/v4'
import {
  MCP_TOOLS,
  MCP_MAX_IMAGE_BYTES,
  MCP_CLIPBOARD_RESOURCE_URI,
  MCP_CLIPBOARD_EVENT_META_KEY,
  type MCPClipboardChange
} from '../../shared/mcp'

export interface MCPClipboardResult {
  data: Record<string, unknown>
  image?: { data: string; mimeType: string }
}

export interface MCPCapabilities {
  getClipboard(includeImage: boolean): Promise<MCPClipboardResult>
  subscribeChanges(listener: (change: MCPClipboardChange) => void): Promise<() => void>
  waitForChange(
    cursor: string | undefined,
    timeoutMs: number,
    signal: AbortSignal
  ): Promise<Record<string, unknown>>
  searchHistory(options: {
    searchText?: string
    limit: number
    offset: number
  }): Promise<Record<string, unknown>>
  getHistoryItem(itemId: string, includeImage: boolean): Promise<MCPClipboardResult>
  setText(text: string): Promise<void>
  recognizeImage(input: { imageBase64?: string; itemId?: string }): Promise<Record<string, unknown>>
}

const readOnly = { readOnlyHint: true, destructiveHint: false, openWorldHint: false }

function result(data: Record<string, unknown>): CallToolResult {
  return { content: [{ type: 'text', text: JSON.stringify(data) }], structuredContent: data }
}

function clipboardResult(value: MCPClipboardResult): CallToolResult {
  const output = result(value.data)
  if (value.image) output.content.push({ type: 'image', ...value.image })
  return output
}

async function execute(action: () => Promise<CallToolResult>): Promise<CallToolResult> {
  try {
    return await action()
  } catch (error) {
    return {
      isError: true,
      content: [{ type: 'text', text: error instanceof Error ? error.message : String(error) }]
    }
  }
}

export function createMCPServer(
  capabilities: MCPCapabilities,
  version: string,
  signal: AbortSignal,
  onError: (error: Error) => void
): McpServer {
  const server = new McpServer(
    { name: 'super-paste', version },
    { capabilities: { resources: { subscribe: true } } }
  )
  let unsubscribe: (() => void) | undefined
  let subscribing: Promise<void> | undefined
  const subscribe = (): Promise<void> => {
    return (subscribing ??= (async () => {
      if (unsubscribe) return
      if (signal.aborted) throw new Error('MCP 会话已关闭')
      const stop = await capabilities.subscribeChanges((change) => {
        void server.server
          .sendResourceUpdated({
            uri: MCP_CLIPBOARD_RESOURCE_URI,
            _meta: { [MCP_CLIPBOARD_EVENT_META_KEY]: change }
          })
          .catch((error) => {
            onError(error)
            unsubscribe?.()
            unsubscribe = undefined
          })
      })
      if (signal.aborted) {
        stop()
        throw new Error('MCP 会话已关闭')
      }
      unsubscribe = stop
    })().finally(() => {
      subscribing = undefined
    }))
  }
  const cancelSubscription = async (): Promise<void> => {
    await subscribing
    unsubscribe?.()
    unsubscribe = undefined
  }
  signal.addEventListener(
    'abort',
    () => {
      unsubscribe?.()
      unsubscribe = undefined
    },
    { once: true }
  )
  server.registerResource(
    'clipboard',
    MCP_CLIPBOARD_RESOURCE_URI,
    {
      title: MCP_TOOLS.get_clipboard.title,
      description: MCP_TOOLS.get_clipboard.description,
      mimeType: 'application/json'
    },
    async () => ({
      contents: [
        {
          uri: MCP_CLIPBOARD_RESOURCE_URI,
          mimeType: 'application/json',
          text: JSON.stringify((await capabilities.getClipboard(false)).data)
        }
      ]
    })
  )
  server.server.setRequestHandler(SubscribeRequestSchema, async ({ params }) => {
    if (params.uri !== MCP_CLIPBOARD_RESOURCE_URI)
      throw new McpError(ErrorCode.InvalidParams, '资源不存在')
    await subscribe()
    return {}
  })
  server.server.setRequestHandler(UnsubscribeRequestSchema, async ({ params }) => {
    if (params.uri !== MCP_CLIPBOARD_RESOURCE_URI)
      throw new McpError(ErrorCode.InvalidParams, '资源不存在')
    await cancelSubscription()
    return {}
  })
  server.registerTool(
    'subscribe_clipboard_changes',
    {
      ...MCP_TOOLS.subscribe_clipboard_changes,
      inputSchema: {},
      annotations: readOnly
    },
    () =>
      execute(async () => {
        await subscribe()
        return result({
          subscribed: true,
          uri: MCP_CLIPBOARD_RESOURCE_URI,
          notification: 'notifications/resources/updated',
          eventMetaKey: MCP_CLIPBOARD_EVENT_META_KEY
        })
      })
  )
  server.registerTool(
    'unsubscribe_clipboard_changes',
    {
      ...MCP_TOOLS.unsubscribe_clipboard_changes,
      inputSchema: {},
      annotations: readOnly
    },
    () =>
      execute(async () => {
        await cancelSubscription()
        return result({ subscribed: false, uri: MCP_CLIPBOARD_RESOURCE_URI })
      })
  )
  server.registerTool(
    'get_clipboard',
    {
      ...MCP_TOOLS.get_clipboard,
      inputSchema: { includeImage: z.boolean().default(false) },
      annotations: readOnly
    },
    ({ includeImage }) =>
      execute(async () => clipboardResult(await capabilities.getClipboard(includeImage)))
  )

  server.registerTool(
    'wait_for_clipboard_change',
    {
      ...MCP_TOOLS.wait_for_clipboard_change,
      inputSchema: {
        cursor: z.string().max(100).optional(),
        timeoutMs: z.number().int().min(0).max(30000).default(25000)
      },
      annotations: readOnly
    },
    ({ cursor, timeoutMs }, extra) =>
      execute(async () =>
        result(
          await capabilities.waitForChange(
            cursor,
            timeoutMs,
            AbortSignal.any([signal, extra.signal])
          )
        )
      )
  )

  server.registerTool(
    'search_clipboard_history',
    {
      ...MCP_TOOLS.search_clipboard_history,
      inputSchema: {
        searchText: z.string().max(1000).optional(),
        limit: z.number().int().min(1).max(100).default(20),
        offset: z.number().int().min(0).max(1000000).default(0)
      },
      annotations: readOnly
    },
    (options) => execute(async () => result(await capabilities.searchHistory(options)))
  )

  server.registerTool(
    'get_clipboard_history_item',
    {
      ...MCP_TOOLS.get_clipboard_history_item,
      inputSchema: { itemId: z.string().min(1).max(256), includeImage: z.boolean().default(false) },
      annotations: readOnly
    },
    ({ itemId, includeImage }) =>
      execute(async () => clipboardResult(await capabilities.getHistoryItem(itemId, includeImage)))
  )

  server.registerTool(
    'set_clipboard_text',
    {
      ...MCP_TOOLS.set_clipboard_text,
      inputSchema: { text: z.string().min(1).max(1000000) },
      annotations: {
        readOnlyHint: false,
        destructiveHint: true,
        idempotentHint: true,
        openWorldHint: false
      }
    },
    ({ text }) =>
      execute(async () => {
        await capabilities.setText(text)
        return result({ success: true })
      })
  )

  server.registerTool(
    'ocr_image',
    {
      ...MCP_TOOLS.ocr_image,
      inputSchema: {
        imageBase64: z
          .string()
          .min(1)
          .max(Math.ceil(MCP_MAX_IMAGE_BYTES / 3) * 4 + 100)
          .optional(),
        itemId: z.string().min(1).max(256).optional()
      },
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true }
    },
    (input) =>
      execute(async () => {
        if (input.imageBase64 && input.itemId) throw new Error('imageBase64 和 itemId 不可同时提供')
        return result(await capabilities.recognizeImage(input))
      })
  )
  return server
}
