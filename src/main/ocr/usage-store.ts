import Database from 'better-sqlite3'
import type {
  AIOCRUsageEntry,
  AIOCRUsageQuery,
  AIOCRUsageStats,
  AIOCRUsageTotals
} from '../../shared/ai-ocr'

const HOUR_MS = 3600000
const FIELDS = [
  'requests',
  'failures',
  'unknownUsage',
  'inputTokens',
  'outputTokens',
  'cachedInputTokens',
  'cacheWriteTokens',
  'totalTokens',
  'durationMs'
] as const
const TOTALS_SQL =
  FIELDS.map((key) => `COALESCE(SUM(${key}), 0) AS ${key}`).join(', ') +
  ', MIN(firstUsedAt) AS firstUsedAt, MAX(lastUsedAt) AS lastUsedAt'
const hourStart = (timestamp: number): number => {
  const date = new Date(timestamp)
  date.setMinutes(0, 0, 0)
  return date.getTime()
}
const dayOf = (timestamp: number): string => {
  const date = new Date(timestamp)
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`
}
const entryTotals = (entry: AIOCRUsageEntry): AIOCRUsageTotals => ({
  requests: 1,
  failures: Number(!entry.success),
  unknownUsage: Number(entry.totalTokens === null),
  inputTokens: entry.inputTokens ?? 0,
  outputTokens: entry.outputTokens ?? 0,
  cachedInputTokens: entry.cachedInputTokens,
  cacheWriteTokens: entry.cacheWriteTokens,
  totalTokens: entry.totalTokens ?? 0,
  durationMs: entry.durationMs,
  firstUsedAt: entry.startedAt,
  lastUsedAt: entry.finishedAt
})
const merge = (target: AIOCRUsageTotals, value: AIOCRUsageTotals): void => {
  for (const key of FIELDS) target[key] += value[key]
  if (value.firstUsedAt !== null)
    target.firstUsedAt = Math.min(target.firstUsedAt ?? Infinity, value.firstUsedAt)
  if (value.lastUsedAt !== null)
    target.lastUsedAt = Math.max(target.lastUsedAt ?? 0, value.lastUsedAt)
}

/** 小时/模型聚合保留完整用量，只保留最近 50 条请求明细。 */
export class AIOCRUsageStore {
  private readonly db: Database.Database
  private readonly pending = new Map<string, AIOCRUsageEntry>()
  private recentIds: Set<string>
  private timer?: ReturnType<typeof setTimeout>
  private clearedAt: number | null

  constructor(
    file: string,
    private readonly options: { flushDelayMs?: number; onError?: (error: unknown) => void } = {}
  ) {
    this.db = new Database(file)
    this.db.pragma('auto_vacuum = INCREMENTAL')
    this.db.pragma('journal_mode = WAL')
    this.db.pragma('cache_size = -256')
    this.db.pragma('journal_size_limit = 524288')
    this.db.pragma('wal_autocheckpoint = 128')
    this.db.exec(`CREATE TABLE IF NOT EXISTS ai_ocr_usage (
      id TEXT PRIMARY KEY, provider TEXT NOT NULL, baseUrl TEXT NOT NULL, model TEXT NOT NULL,
      purpose TEXT NOT NULL, startedAt INTEGER NOT NULL, finishedAt INTEGER NOT NULL,
      durationMs INTEGER NOT NULL, success INTEGER NOT NULL, inputTokens INTEGER,
      outputTokens INTEGER, cachedInputTokens INTEGER NOT NULL, cacheWriteTokens INTEGER NOT NULL,
      totalTokens INTEGER
    );
    CREATE INDEX IF NOT EXISTS ai_ocr_usage_time ON ai_ocr_usage(startedAt);
    CREATE TABLE IF NOT EXISTS ai_ocr_usage_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS ai_ocr_models (
      id INTEGER PRIMARY KEY, provider TEXT NOT NULL, baseUrl TEXT NOT NULL, model TEXT NOT NULL,
      UNIQUE(provider, baseUrl, model)
    );
    CREATE TABLE IF NOT EXISTS ai_ocr_usage_hourly (
      hour INTEGER NOT NULL, modelId INTEGER NOT NULL,
      requests INTEGER NOT NULL, failures INTEGER NOT NULL, unknownUsage INTEGER NOT NULL,
      inputTokens INTEGER NOT NULL, outputTokens INTEGER NOT NULL, cachedInputTokens INTEGER NOT NULL,
      cacheWriteTokens INTEGER NOT NULL, totalTokens INTEGER NOT NULL, durationMs INTEGER NOT NULL,
      firstUsedAt INTEGER NOT NULL, lastUsedAt INTEGER NOT NULL,
      PRIMARY KEY(hour, modelId)
    ) WITHOUT ROWID;`)
    this.db.function('statistics_hour', { deterministic: true }, hourStart)
    // 迁移标识与聚合写入在同一事务中，重启不会再次累加旧明细。
    if (!this.db.prepare("SELECT 1 FROM ai_ocr_usage_meta WHERE key = 'hourlyVersion'").get()) {
      this.db.transaction(() => {
        this.db.exec(`INSERT OR IGNORE INTO ai_ocr_models(provider, baseUrl, model)
          SELECT DISTINCT provider, baseUrl, model FROM ai_ocr_usage;
          INSERT INTO ai_ocr_usage_hourly
          SELECT statistics_hour(u.startedAt), m.id, COUNT(*), SUM(u.success = 0), SUM(u.totalTokens IS NULL),
            COALESCE(SUM(u.inputTokens), 0), COALESCE(SUM(u.outputTokens), 0), SUM(u.cachedInputTokens),
            SUM(u.cacheWriteTokens), COALESCE(SUM(u.totalTokens), 0), SUM(u.durationMs), MIN(u.startedAt), MAX(u.finishedAt)
          FROM ai_ocr_usage u JOIN ai_ocr_models m USING(provider, baseUrl, model)
          GROUP BY statistics_hour(u.startedAt), m.id;
          DELETE FROM ai_ocr_usage WHERE id NOT IN
            (SELECT id FROM ai_ocr_usage ORDER BY startedAt DESC, id DESC LIMIT 50);
          INSERT INTO ai_ocr_usage_meta VALUES ('hourlyVersion', 1);`)
      })()
    }
    this.clearedAt =
      (
        this.db.prepare("SELECT value FROM ai_ocr_usage_meta WHERE key = 'clearedAt'").get() as
          | { value: number }
          | undefined
      )?.value ?? null
    this.recentIds = this.readRecentIds()
  }

  private readRecentIds(): Set<string> {
    return new Set(
      (this.db.prepare('SELECT id FROM ai_ocr_usage').all() as { id: string }[]).map(
        (row) => row.id
      )
    )
  }

  public record(entry: AIOCRUsageEntry): void {
    if (this.clearedAt !== null && entry.startedAt <= this.clearedAt) return
    if (this.pending.has(entry.id) || this.recentIds.has(entry.id)) return
    // 正常路径只更新内存，存储故障时也不允许缓存无限增长。
    if (this.pending.size >= 256) this.flush()
    this.pending.set(entry.id, { ...entry })
    this.scheduleFlush()
  }

  private scheduleFlush(): void {
    if (this.timer) return
    this.timer = setTimeout(() => {
      this.timer = undefined
      try {
        this.flush()
      } catch (error) {
        this.options.onError?.(error)
        this.scheduleFlush()
      }
    }, this.options.flushDelayMs ?? 10000)
    this.timer.unref()
  }

  public flush(): void {
    if (!this.pending.size) return
    const insert = this.db.prepare(`INSERT OR IGNORE INTO ai_ocr_usage
      VALUES (@id, @provider, @baseUrl, @model, @purpose, @startedAt, @finishedAt, @durationMs,
        @success, @inputTokens, @outputTokens, @cachedInputTokens, @cacheWriteTokens, @totalTokens)`)
    const addModel = this.db.prepare(
      'INSERT OR IGNORE INTO ai_ocr_models(provider, baseUrl, model) VALUES (?, ?, ?)'
    )
    const getModel = this.db.prepare(
      'SELECT id FROM ai_ocr_models WHERE provider = ? AND baseUrl = ? AND model = ?'
    )
    const aggregate = this.db.prepare(`INSERT INTO ai_ocr_usage_hourly
      VALUES (@hour, @modelId, ${FIELDS.map((field) => `@${field}`).join(', ')}, @firstUsedAt, @lastUsedAt)
      ON CONFLICT(hour, modelId) DO UPDATE SET
        ${FIELDS.map((field) => `${field} = ${field} + excluded.${field}`).join(', ')},
        firstUsedAt = MIN(firstUsedAt, excluded.firstUsedAt), lastUsedAt = MAX(lastUsedAt, excluded.lastUsedAt)`)
    this.db.transaction(() => {
      const models = new Map<string, number>()
      for (const entry of this.pending.values()) {
        if (!insert.run({ ...entry, success: Number(entry.success) }).changes) continue
        const key = JSON.stringify([entry.provider, entry.baseUrl, entry.model])
        let modelId = models.get(key)
        if (modelId === undefined) {
          addModel.run(entry.provider, entry.baseUrl, entry.model)
          modelId = (getModel.get(entry.provider, entry.baseUrl, entry.model) as { id: number }).id
          models.set(key, modelId)
        }
        aggregate.run({ hour: hourStart(entry.startedAt), modelId, ...entryTotals(entry) })
      }
      this.db.exec(`DELETE FROM ai_ocr_usage WHERE id NOT IN
        (SELECT id FROM ai_ocr_usage ORDER BY startedAt DESC, id DESC LIMIT 50);`)
    })()
    this.pending.clear()
    this.recentIds = this.readRecentIds()
  }

  public getStats(query: AIOCRUsageQuery = {}): AIOCRUsageStats {
    for (const value of [query.from, query.to])
      if (
        value !== undefined &&
        (!Number.isSafeInteger(value) || value < 0 || value > 8640000000000000)
      )
        throw new Error('统计时间范围无效')
    if (query.from !== undefined && query.to !== undefined && query.from > query.to)
      throw new Error('统计开始时间不能晚于结束时间')
    // 与复制统计统一按小时筛选，页面使用自然日边界。
    const from = query.from === undefined ? 0 : hourStart(query.from)
    const to = query.to === undefined ? Number.MAX_SAFE_INTEGER : hourStart(query.to) + HOUR_MS
    const where = 'WHERE hour >= ? AND hour < ?'
    const totals = this.db
      .prepare(`SELECT ${TOTALS_SQL} FROM ai_ocr_usage_hourly ${where}`)
      .get(from, to) as AIOCRUsageTotals
    const models = this.db
      .prepare(
        `SELECT provider, baseUrl, model, ${TOTALS_SQL}
      FROM ai_ocr_usage_hourly JOIN ai_ocr_models ON modelId = ai_ocr_models.id
      ${where} GROUP BY modelId ORDER BY totalTokens DESC`
      )
      .all(from, to) as AIOCRUsageStats['models']
    const days = this.db
      .prepare(
        `SELECT date(hour / 1000, 'unixepoch', 'localtime') AS day, ${TOTALS_SQL}
      FROM ai_ocr_usage_hourly ${where} GROUP BY day ORDER BY day DESC`
      )
      .all(from, to) as AIOCRUsageStats['days']
    const modelMap = new Map(
      models.map((model) => [JSON.stringify([model.provider, model.baseUrl, model.model]), model])
    )
    const dayMap = new Map(days.map((day) => [day.day, day]))
    const buffered = [...this.pending.values()].filter(
      (entry) => hourStart(entry.startedAt) >= from && hourStart(entry.startedAt) < to
    )
    for (const entry of buffered) {
      const value = entryTotals(entry)
      merge(totals, value)
      const key = JSON.stringify([entry.provider, entry.baseUrl, entry.model])
      const model = modelMap.get(key)
      if (model) merge(model, value)
      else
        modelMap.set(key, {
          provider: entry.provider,
          baseUrl: entry.baseUrl,
          model: entry.model,
          ...value
        })
      const day = dayOf(entry.startedAt)
      const dayTotals = dayMap.get(day)
      if (dayTotals) merge(dayTotals, value)
      else dayMap.set(day, { day, ...value })
    }
    const recent = this.db
      .prepare(
        `SELECT * FROM ai_ocr_usage WHERE startedAt >= ? AND startedAt < ?
      ORDER BY startedAt DESC, id DESC LIMIT 50`
      )
      .all(from, to) as (Omit<AIOCRUsageEntry, 'success'> & { success: number })[]
    return {
      clearedAt: this.clearedAt,
      totals,
      models: [...modelMap.values()].sort((a, b) => b.totalTokens - a.totalTokens),
      days: [...dayMap.values()].sort((a, b) => b.day.localeCompare(a.day)),
      recent: [
        ...recent.map((entry) => ({ ...entry, success: Boolean(entry.success) })),
        ...buffered
      ]
        .sort((a, b) => b.startedAt - a.startedAt || b.id.localeCompare(a.id))
        .slice(0, 50)
    }
  }

  public clear(): AIOCRUsageStats {
    const clearedAt = Date.now()
    this.db.transaction(() => {
      this.db.exec(
        'DELETE FROM ai_ocr_usage; DELETE FROM ai_ocr_usage_hourly; DELETE FROM ai_ocr_models;'
      )
      this.db
        .prepare("INSERT OR REPLACE INTO ai_ocr_usage_meta (key, value) VALUES ('clearedAt', ?)")
        .run(clearedAt)
    })()
    this.pending.clear()
    this.recentIds.clear()
    this.clearedAt = clearedAt
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    this.db.pragma('incremental_vacuum(128)')
    return this.getStats()
  }

  public close(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    this.flush()
    this.db.close()
  }
}
