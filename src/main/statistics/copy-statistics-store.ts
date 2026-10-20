import Database from 'better-sqlite3'
import {
  COPY_STATISTICS_TYPES,
  type CopyStatistics,
  type CopyStatisticsEvent,
  type CopyStatisticsGroup,
  type CopyStatisticsQuery,
  type CopyStatisticsTotals,
  type StatisticsInterval
} from '../../shared/clipboard-statistics'

const HOUR = 3600000
const SUM_SQL = `SUM(copies) AS copies, SUM(bytes) AS bytes, SUM(files) AS files,
  MIN(firstCopiedAt) AS firstCopiedAt, MAX(lastCopiedAt) AS lastCopiedAt`
const emptyTotals = (): CopyStatisticsTotals => ({
  copies: 0,
  bytes: 0,
  files: 0,
  firstCopiedAt: null,
  lastCopiedAt: null
})
type Bucket = CopyStatisticsTotals & { hour: number; type: string; source: string }
const hourStart = (timestamp: number): number => {
  const date = new Date(timestamp)
  date.setMinutes(0, 0, 0)
  return date.getTime()
}
const periodKey = (timestamp: number, interval: StatisticsInterval): string => {
  if (interval === 'hour') return String(hourStart(timestamp))
  const date = new Date(timestamp)
  const month = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`
  return interval === 'month' ? month : `${month}-${String(date.getDate()).padStart(2, '0')}`
}
const merge = (target: CopyStatisticsTotals, value: CopyStatisticsTotals): void => {
  target.copies += value.copies
  target.bytes += value.bytes
  target.files += value.files
  if (value.firstCopiedAt !== null)
    target.firstCopiedAt = Math.min(target.firstCopiedAt ?? Infinity, value.firstCopiedAt)
  if (value.lastCopiedAt !== null)
    target.lastCopiedAt = Math.max(target.lastCopiedAt ?? 0, value.lastCopiedAt)
}
const mergeGroup = (
  map: Map<string, CopyStatisticsGroup>,
  key: string,
  value: CopyStatisticsTotals
): void => {
  let group = map.get(key)
  if (!group) map.set(key, (group = { key, ...emptyTotals() }))
  merge(group, value)
}

/** 只保留聚合计数，不保存剪贴板正文，也不依赖历史记录的生命周期。 */
export class CopyStatisticsStore {
  private readonly db: Database.Database
  private readonly pending = new Map<string, Bucket>()
  private timer?: ReturnType<typeof setTimeout>
  private clearedAt: number | null

  constructor(
    file: string,
    private readonly options: {
      flushDelayMs?: number
      onError?: (error: unknown) => void
    } = {}
  ) {
    this.db = new Database(file)
    this.db.pragma('auto_vacuum = INCREMENTAL')
    this.db.pragma('journal_mode = WAL')
    this.db.pragma('cache_size = -256')
    this.db.pragma('journal_size_limit = 524288')
    this.db.pragma('wal_autocheckpoint = 128')
    this.db.exec(`CREATE TABLE IF NOT EXISTS copy_statistics (
      hour INTEGER NOT NULL, type TEXT NOT NULL, source TEXT NOT NULL,
      copies INTEGER NOT NULL, bytes INTEGER NOT NULL, files INTEGER NOT NULL,
      firstCopiedAt INTEGER NOT NULL, lastCopiedAt INTEGER NOT NULL,
      PRIMARY KEY (hour, type, source)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS copy_statistics_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);`)
    this.clearedAt =
      (
        this.db.prepare("SELECT value FROM copy_statistics_meta WHERE key = 'clearedAt'").get() as
          | { value: number }
          | undefined
      )?.value ?? null
  }

  public record(event: CopyStatisticsEvent): boolean {
    if (
      !Number.isSafeInteger(event.timestamp) ||
      event.timestamp < 0 ||
      event.timestamp > 8640000000000000 ||
      !COPY_STATISTICS_TYPES.includes(event.type) ||
      typeof event.source !== 'string' ||
      event.source.length > 200 ||
      !Number.isSafeInteger(event.bytes) ||
      event.bytes < 0 ||
      !Number.isSafeInteger(event.files) ||
      event.files < 0
    )
      throw new Error('复制统计事件无效')
    // 丢弃清空之前已采集、但尚未从 Worker 送达的事件。
    if (this.clearedAt !== null && event.timestamp <= this.clearedAt) return false
    const hour = hourStart(event.timestamp)
    const key = JSON.stringify([hour, event.type, event.source])
    let bucket = this.pending.get(key)
    if (!bucket && this.pending.size >= 256) this.flush()
    if (!bucket)
      this.pending.set(
        key,
        (bucket = {
          hour,
          type: event.type,
          source: event.source,
          ...emptyTotals()
        })
      )
    merge(bucket, {
      copies: 1,
      bytes: event.bytes,
      files: event.files,
      firstCopiedAt: event.timestamp,
      lastCopiedAt: event.timestamp
    })
    // 从首条事件开始计时，持续复制也不会无限推迟持久化。
    this.scheduleFlush()
    return true
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
    const statement = this.db.prepare(`INSERT INTO copy_statistics
      (hour, type, source, copies, bytes, files, firstCopiedAt, lastCopiedAt)
      VALUES (@hour, @type, @source, @copies, @bytes, @files, @firstCopiedAt, @lastCopiedAt)
      ON CONFLICT(hour, type, source) DO UPDATE SET
        copies = copies + excluded.copies, bytes = bytes + excluded.bytes,
        files = files + excluded.files,
        firstCopiedAt = MIN(firstCopiedAt, excluded.firstCopiedAt),
        lastCopiedAt = MAX(lastCopiedAt, excluded.lastCopiedAt)`)
    this.db.transaction(() => {
      for (const bucket of this.pending.values()) statement.run(bucket)
    })()
    // 事务失败时保留原始增量，下一轮继续重试。
    this.pending.clear()
  }

  public getStats(value: unknown = {}): CopyStatistics {
    if (!value || typeof value !== 'object' || Array.isArray(value))
      throw new Error('复制统计查询参数无效')
    const query = value as CopyStatisticsQuery
    for (const timestamp of [query.from, query.to])
      if (
        timestamp !== undefined &&
        (!Number.isSafeInteger(timestamp) || timestamp < 0 || timestamp > 8640000000000000)
      )
        throw new Error('统计时间范围无效')
    if (query.type !== undefined && !COPY_STATISTICS_TYPES.includes(query.type))
      throw new Error('统计类型无效')
    if (
      query.source !== undefined &&
      (typeof query.source !== 'string' || query.source.length > 200)
    )
      throw new Error('统计来源无效')
    const interval = query.interval ?? 'day'
    if (!['hour', 'day', 'month'].includes(interval)) throw new Error('统计时间粒度无效')
    const from = query.from === undefined ? 0 : hourStart(query.from)
    const to = query.to ?? Number.MAX_SAFE_INTEGER
    if ((query.from ?? 0) >= to) throw new Error('统计开始时间必须早于结束时间')
    const conditions = ['hour >= @from', 'hour < @to']
    const params: Record<string, number | string> = { from, to }
    if (query.type !== undefined) {
      conditions.push('type = @type')
      params.type = query.type
    }
    const sourcesWhere = conditions.join(' AND ')
    const availableSources = new Set(
      (
        this.db
          .prepare(`SELECT DISTINCT source FROM copy_statistics WHERE ${sourcesWhere}`)
          .all(params) as { source: string }[]
      ).map((row) => row.source)
    )
    if (query.source !== undefined) {
      conditions.push('source = @source')
      params.source = query.source
    }
    const where = conditions.join(' AND ')
    const readGroups = (expression: string): Map<string, CopyStatisticsGroup> =>
      new Map(
        (
          this.db
            .prepare(
              `SELECT ${expression} AS key, ${SUM_SQL}
        FROM copy_statistics WHERE ${where} GROUP BY key`
            )
            .all(params) as CopyStatisticsGroup[]
        ).map((group) => [group.key, group])
      )
    const timeExpression = (unit: StatisticsInterval): string =>
      unit === 'hour'
        ? 'CAST(hour AS TEXT)'
        : `strftime('${unit === 'day' ? '%Y-%m-%d' : '%Y-%m'}', hour / 1000, 'unixepoch', 'localtime')`
    const types = readGroups('type')
    const sources = readGroups('source')
    const days = readGroups(timeExpression('day'))
    const periods = interval === 'day' ? days : readGroups(timeExpression(interval))
    for (const bucket of this.pending.values()) {
      if (bucket.hour < from || bucket.hour >= to || (query.type && query.type !== bucket.type))
        continue
      availableSources.add(bucket.source)
      if (query.source !== undefined && query.source !== bucket.source) continue
      mergeGroup(types, bucket.type, bucket)
      mergeGroup(sources, bucket.source, bucket)
      mergeGroup(days, periodKey(bucket.hour, 'day'), bucket)
      if (periods !== days) mergeGroup(periods, periodKey(bucket.hour, interval), bucket)
    }
    const totals = emptyTotals()
    for (const day of days.values()) merge(totals, day)
    const start = query.from ?? totals.firstCopiedAt ?? Date.now()
    const end = Math.min(to, Date.now() + 1)
    const firstKey = periodKey(start, interval)
    const cursor = new Date(
      interval === 'hour'
        ? Number(firstKey)
        : `${firstKey}${interval === 'month' ? '-01' : ''}T00:00:00`
    )
    const series: CopyStatistics['series'] = []
    while (cursor.getTime() < end) {
      if (series.length >= 2000) throw new Error('时间范围过长，请选择更大的时间粒度')
      const timestamp = cursor.getTime()
      const key = periodKey(timestamp, interval)
      series.push({ key, timestamp, ...(periods.get(key) ?? emptyTotals()) })
      if (interval === 'hour') cursor.setTime(timestamp + HOUR)
      else if (interval === 'day') cursor.setDate(cursor.getDate() + 1)
      else cursor.setMonth(cursor.getMonth() + 1)
    }
    const sort = (map: Map<string, CopyStatisticsGroup>): CopyStatisticsGroup[] =>
      [...map.values()].sort((a, b) => b.copies - a.copies || a.key.localeCompare(b.key))
    return {
      ...totals,
      clearedAt: this.clearedAt,
      activeDays: days.size,
      types: sort(types),
      sources: sort(sources),
      availableSources: [...availableSources].sort(),
      series,
      interval
    }
  }

  public clear(): void {
    const timestamp = Date.now()
    this.db.transaction(() => {
      this.db.prepare('DELETE FROM copy_statistics').run()
      this.db
        .prepare("INSERT OR REPLACE INTO copy_statistics_meta (key, value) VALUES ('clearedAt', ?)")
        .run(timestamp)
    })()
    this.clearedAt = timestamp
    this.pending.clear()
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    this.db.pragma('incremental_vacuum(128)')
  }

  public close(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = undefined
    this.flush()
    this.db.close()
  }
}
