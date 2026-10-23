import type { OCRTaskStatus } from '../../shared/ai-ocr'

export interface OCRJob {
  imageHash: string
  records: Map<string, { automatic: boolean }>
}

export class OCRTaskQueue {
  private readonly jobs = new Map<string, OCRJob>()
  private pending: OCRJob[] = []
  private running: Promise<void> | null = null
  private paused = true
  private active: OCRJob | undefined

  constructor(
    private readonly run: (job: OCRJob) => Promise<void>,
    private readonly onStatus: (status: OCRTaskStatus) => void,
    private readonly onIdle: () => void,
    private readonly limit = 10000
  ) {}

  public enqueue(imageHash: string, contentId: string, automatic: boolean): boolean {
    let job = this.jobs.get(imageHash)
    const alreadyQueued = job?.records.has(contentId) || false
    if (!job) {
      if (this.jobs.size >= this.limit) throw new Error('OCR 等待队列已满，请稍后重试')
      job = { imageHash, records: new Map() }
      this.jobs.set(imageHash, job)
      this.pending.push(job)
    }
    const previous = job.records.get(contentId)
    job.records.set(contentId, {
      automatic: previous ? previous.automatic && automatic : automatic
    })
    if (!alreadyQueued)
      this.onStatus({ contentId, state: job === this.active ? 'running' : 'queued' })
    this.drain()
    return alreadyQueued
  }

  public resume(): void {
    this.paused = false
    this.drain()
  }

  public get hasJobs(): boolean {
    return this.jobs.size > 0
  }

  public async pause(): Promise<void> {
    this.paused = true
    await this.running
  }

  public cancelAutomatic(): void {
    for (const job of this.pending) {
      for (const [id, record] of job.records) {
        if (!record.automatic) continue
        job.records.delete(id)
        this.onStatus({ contentId: id, state: 'cancelled' })
      }
      if (!job.records.size) this.jobs.delete(job.imageHash)
    }
    this.pending = this.pending.filter((job) => job.records.size)
    if (!this.hasJobs) this.onIdle()
  }

  public failPending(error: string): void {
    for (const job of this.pending) {
      for (const id of job.records.keys()) this.onStatus({ contentId: id, state: 'error', error })
      this.jobs.delete(job.imageHash)
    }
    this.pending = []
    if (!this.hasJobs) this.onIdle()
  }

  private drain(): void {
    if (this.paused || this.running || !this.pending.length) return
    const job = this.pending.shift()!
    this.active = job
    // 下一轮事件循环开始识别，让提交 OCR 的 IPC 和剪贴板保存先返回。
    this.running = new Promise<void>((resolve) => setImmediate(resolve))
      .then(async () => {
        for (const id of job.records.keys()) this.onStatus({ contentId: id, state: 'running' })
        await this.run(job)
        for (const id of job.records.keys()) this.onStatus({ contentId: id, state: 'completed' })
      })
      .catch((error) => {
        for (const id of job.records.keys())
          this.onStatus({
            contentId: id,
            state: 'error',
            error: error instanceof Error ? error.message : String(error)
          })
      })
      .finally(() => {
        this.jobs.delete(job.imageHash)
        this.active = undefined
        this.running = null
        this.drain()
        if (!this.hasJobs) this.onIdle()
      })
  }
}
