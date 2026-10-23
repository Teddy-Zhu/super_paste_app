import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http'
import { createHash, timingSafeEqual, randomUUID } from 'node:crypto'
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { isInitializeRequest } from '@modelcontextprotocol/sdk/types.js'
import { MCP_HOST, MCP_PATH, type MCPSettings } from '../../shared/mcp'

const MAX_BODY_BYTES = 15 * 1024 * 1024
const MAX_REQUESTS = 32
const MAX_SESSIONS = 16
const SESSION_IDLE_MS = 5 * 60000

interface MCPSession {
  id: string
  mcp: McpServer
  transport: StreamableHTTPServerTransport
  controller: AbortController
  activeRequests: number
  idleTimer?: NodeJS.Timeout
}

function respond(response: ServerResponse, status: number, message: string, code = -32000): void {
  response.writeHead(status, { 'Content-Type': 'application/json' })
  response.end(JSON.stringify({ jsonrpc: '2.0', id: null, error: { code, message } }))
}

export class MCPHttpServer {
  private server: Server | undefined
  private readonly requests = new Set<AbortController>()
  private readonly sessions = new Map<string, MCPSession>()

  constructor(
    private readonly createMCP: (signal: AbortSignal) => McpServer,
    private readonly onError: (error: Error) => void
  ) {}

  public get running(): boolean {
    return this.server?.listening ?? false
  }

  public async start(settings: MCPSettings): Promise<void> {
    if (this.server) throw new Error('MCP 服务已经启动')
    const server = createServer((request, response) => {
      void this.handle(request, response, settings).catch(() => {
        if (!response.headersSent) respond(response, 500, 'MCP 请求处理失败', -32603)
        else response.destroy()
      })
    })
    server.headersTimeout = 10000
    server.requestTimeout = 15000
    await new Promise<void>((resolve, reject) => {
      const onError = (error: NodeJS.ErrnoException): void => {
        server.close()
        reject(
          new Error(
            error.code === 'EADDRINUSE'
              ? `端口 ${settings.port} 已被占用，请更换 MCP 端口`
              : `MCP 服务启动失败：${error.message}`
          )
        )
      }
      server.once('error', onError)
      server.listen(settings.port, MCP_HOST, () => {
        server.off('error', onError)
        resolve()
      })
    })
    server.on('error', this.onError)
    this.server = server
  }

  public async stop(): Promise<void> {
    const server = this.server
    this.server = undefined
    if (!server) return
    for (const request of this.requests) request.abort()
    await Promise.all([...this.sessions.values()].map((session) => this.closeSession(session)))
    const closed = new Promise<void>((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()))
    })
    // SSE、等待调用和 HTTP keep-alive 都不能阻止退出或更换端口。
    server.closeAllConnections()
    await closed
  }

  private async handle(
    request: IncomingMessage,
    response: ServerResponse,
    settings: MCPSettings
  ): Promise<void> {
    response.setHeader('Cache-Control', 'no-store')
    response.setHeader('X-Content-Type-Options', 'nosniff')
    const hosts = [`${MCP_HOST}:${settings.port}`, `localhost:${settings.port}`]
    if (!hosts.includes(request.headers.host ?? '')) {
      respond(response, 403, '不允许的 Host')
      return
    }
    const origin = request.headers.origin
    if (origin && !hosts.some((host) => origin === `http://${host}`)) {
      respond(response, 403, '不允许的 Origin')
      return
    }
    if (request.url !== MCP_PATH) {
      respond(response, 404, '接口不存在')
      return
    }
    if (settings.authEnabled) {
      const supplied = request.headers.authorization ?? ''
      const expected = `Bearer ${settings.apiKey}`
      const hash = (value: string): Buffer => createHash('sha256').update(value).digest()
      if (!timingSafeEqual(hash(supplied), hash(expected))) {
        response.setHeader('WWW-Authenticate', 'Bearer realm="Super Paste MCP"')
        respond(response, 401, 'API Key 缺失或无效')
        return
      }
    }
    if (!['POST', 'GET', 'DELETE'].includes(request.method ?? '')) {
      response.setHeader('Allow', 'POST, GET, DELETE')
      respond(response, 405, '请使用 MCP Streamable HTTP POST、GET 或 DELETE 请求')
      return
    }
    if (
      request.method === 'POST' &&
      !/^application\/json(?:\s*;|$)/i.test(request.headers['content-type'] ?? '')
    ) {
      respond(response, 415, 'Content-Type 须为 application/json')
      return
    }
    if (this.requests.size >= MAX_REQUESTS) {
      respond(response, 429, 'MCP 并发请求过多，请稍后重试')
      return
    }
    const controller = new AbortController()
    this.requests.add(controller)
    let session: MCPSession | undefined
    const onClose = (): void => controller.abort()
    response.once('close', onClose)
    try {
      let body: unknown
      if (request.method === 'POST') {
        const chunks: Buffer[] = []
        let size = 0
        request.setTimeout(10000, () => request.destroy())
        for await (const chunk of request) {
          size += chunk.length
          if (size > MAX_BODY_BYTES) {
            respond(response, 413, '请求体超过 15 MiB 限制')
            return
          }
          chunks.push(Buffer.from(chunk))
        }
        request.setTimeout(0)
        try {
          body = JSON.parse(Buffer.concat(chunks).toString('utf8'))
        } catch {
          respond(response, 400, 'JSON 解析失败', -32700)
          return
        }
      }
      if (controller.signal.aborted) return
      const sessionId = request.headers['mcp-session-id']
      if (typeof sessionId === 'string') {
        session = this.sessions.get(sessionId)
        if (!session) {
          respond(response, 404, 'MCP 会话已失效，请重新初始化')
          return
        }
      } else if (
        sessionId === undefined &&
        request.method === 'POST' &&
        isInitializeRequest(body)
      ) {
        if (this.sessions.size >= MAX_SESSIONS) {
          respond(response, 429, 'MCP 会话过多，请关闭闲置连接')
          return
        }
        session = await this.createSession()
      } else {
        respond(response, 400, '缺少有效的 Mcp-Session-Id，请先初始化 MCP 会话')
        return
      }
      clearTimeout(session.idleTimer)
      session.activeRequests++
      await session.transport.handleRequest(request, response, body)
    } finally {
      controller.abort()
      response.off('close', onClose)
      this.requests.delete(controller)
      if (session) {
        session.activeRequests--
        if (!session.transport.sessionId) await this.closeSession(session)
        else this.scheduleSessionExpiry(session)
      }
    }
  }

  private async createSession(): Promise<MCPSession> {
    const controller = new AbortController()
    const id = randomUUID()
    const mcp = this.createMCP(controller.signal)
    const transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => id,
      enableJsonResponse: true
    })
    const session: MCPSession = { id, controller, mcp, transport, activeRequests: 0 }
    mcp.server.onclose = () => {
      controller.abort()
      clearTimeout(session.idleTimer)
      this.sessions.delete(id)
    }
    mcp.server.onerror = this.onError
    this.sessions.set(id, session)
    try {
      await mcp.connect(transport)
    } catch (error) {
      await this.closeSession(session)
      throw error
    }
    return session
  }

  private scheduleSessionExpiry(session: MCPSession): void {
    if (session.controller.signal.aborted || session.activeRequests) return
    clearTimeout(session.idleTimer)
    session.idleTimer = setTimeout(() => {
      void this.closeSession(session).catch(this.onError)
    }, SESSION_IDLE_MS)
    session.idleTimer.unref()
  }

  private async closeSession(session: MCPSession): Promise<void> {
    session.controller.abort()
    clearTimeout(session.idleTimer)
    this.sessions.delete(session.id)
    await session.mcp.close()
  }
}
