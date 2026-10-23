import { app } from 'electron'
import {
  createDefaultMCPSettings,
  getMCPEndpoint,
  type MCPState,
  type MCPStatus
} from '../../shared/mcp'
import { loadMCPSettings, parseMCPSettings, persistMCPSettings } from './settings'
import { createMCPServer } from './tools'
import { AppMCPCapabilities } from './capabilities'
import { MCPHttpServer } from './http-server'
import { NotificationService } from '../services/notification-service'
import logger from '../utils/logger'

class MCPService {
  private settings = createDefaultMCPSettings()
  private capabilities: AppMCPCapabilities | undefined
  private http: MCPHttpServer | undefined
  private error: string | undefined
  private queue: Promise<unknown> = Promise.resolve()
  private initialized: Promise<void> | undefined
  private disposed = false

  public initialize(): Promise<void> {
    return (this.initialized ??= this.enqueue(async () => {
      try {
        this.settings = loadMCPSettings()
        if (this.settings.enabled) await this.start()
      } catch (error) {
        this.reportError(error)
      }
      this.notify()
    }))
  }

  public async getState(): Promise<MCPState> {
    await this.initialize()
    await this.queue
    return { settings: { ...this.settings }, status: this.getStatus() }
  }

  public getStatus(): MCPStatus {
    return {
      running: this.http?.running ?? false,
      endpoint: getMCPEndpoint(this.settings.port),
      error: this.error
    }
  }

  public async save(value: unknown): Promise<MCPState> {
    const next = parseMCPSettings(value)
    await this.initialize()
    return this.enqueue(async () => {
      if (this.disposed) throw new Error('应用正在退出')
      const previous = this.settings
      await this.stop()
      this.settings = next
      try {
        if (next.enabled) await this.start()
        persistMCPSettings(next)
        this.error = undefined
      } catch (error) {
        await this.stop()
        this.settings = previous
        let failure = error
        try {
          if (previous.enabled) await this.start()
        } catch (rollbackError) {
          failure = new Error(
            `${error instanceof Error ? error.message : String(error)}；恢复原配置失败：${rollbackError instanceof Error ? rollbackError.message : String(rollbackError)}`
          )
        }
        this.reportError(failure)
        this.notify()
        throw failure
      }
      this.notify()
      return { settings: { ...this.settings }, status: this.getStatus() }
    })
  }

  public async dispose(): Promise<void> {
    this.disposed = true
    await this.enqueue(() => this.stop())
  }

  private enqueue<T>(action: () => Promise<T>): Promise<T> {
    const result = this.queue.then(action)
    this.queue = result.catch(() => undefined)
    return result
  }

  private async start(): Promise<void> {
    if (this.disposed) throw new Error('应用正在退出')
    const capabilities = new AppMCPCapabilities()
    const http = new MCPHttpServer(
      (signal) =>
        createMCPServer(capabilities, app.getVersion(), signal, (error) =>
          logger.error('[MCP] 推送失败:', error)
        ),
      (error) => {
        this.reportError(error)
        this.notify()
      }
    )
    try {
      await http.start(this.settings)
      this.capabilities = capabilities
      this.http = http
    } catch (error) {
      await capabilities.stop()
      throw error
    }
  }

  private async stop(): Promise<void> {
    await this.http?.stop()
    this.http = undefined
    await this.capabilities?.stop()
    this.capabilities = undefined
  }

  private reportError(error: unknown): void {
    this.error = error instanceof Error ? error.message : String(error)
    logger.error('[MCP]', this.error)
  }

  private notify(): void {
    NotificationService.notifyFrontend('mcp:status', this.getStatus())
  }
}

export const mcpService = new MCPService()
