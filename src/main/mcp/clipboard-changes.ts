import { randomUUID } from 'node:crypto'
import { EventEmitter } from 'node:events'
import type { ClipboardChangeData, MCPClipboardChange } from '../../shared/mcp'

export class ClipboardChanges {
  private readonly events = new EventEmitter().setMaxListeners(48)
  private readonly instanceId = randomUUID()
  private version = 0
  private timestamp: number | null = null

  public get cursor(): string {
    return `${this.instanceId}:${this.version}`
  }

  public record(change: ClipboardChangeData): void {
    this.timestamp = change.timestamp
    this.version++
    this.events.emit('change', { ...change, cursor: this.cursor })
  }

  public subscribe(listener: (change: MCPClipboardChange) => void): () => void {
    this.events.on('change', listener)
    return () => this.events.off('change', listener)
  }

  public async wait(
    cursor: string | undefined,
    timeoutMs: number,
    signal: AbortSignal
  ): Promise<{
    changed: boolean
    cursor: string
    timestamp: number | null
  }> {
    const initial = cursor ?? this.cursor
    const [instance, version] = initial.split(':')
    if (
      instance !== this.instanceId ||
      !/^\d+$/.test(version ?? '') ||
      Number(version) > this.version ||
      initial !== `${instance}:${Number(version)}`
    )
      throw new Error('剪贴板 cursor 已失效，请调用 get_clipboard 获取新的 cursor')
    if (signal.aborted) throw new Error('请求已取消')
    if (initial === this.cursor && timeoutMs > 0) {
      await new Promise<void>((resolve, reject) => {
        const finish = (): void => {
          clearTimeout(timer)
          this.events.off('change', onChange)
          signal.removeEventListener('abort', onAbort)
        }
        const onChange = (): void => {
          finish()
          resolve()
        }
        const onAbort = (): void => {
          finish()
          reject(new Error('请求已取消'))
        }
        const timer = setTimeout(onChange, timeoutMs)
        this.events.on('change', onChange)
        signal.addEventListener('abort', onAbort, { once: true })
      })
    }
    return { changed: initial !== this.cursor, cursor: this.cursor, timestamp: this.timestamp }
  }
}
