import { STATISTICS_BYTE_UNITS } from '@renderer/constants/statistics'

export const formatStatisticsBytes = (value: number): string => {
  const unit = value > 0 ? Math.min(Math.floor(Math.log(value) / Math.log(1024)), 4) : 0
  return `${(value / 1024 ** unit).toLocaleString(undefined, { maximumFractionDigits: unit ? 2 : 0 })} ${STATISTICS_BYTE_UNITS[unit]}`
}

export interface StatisticsChartPoint {
  timestamp: number
  value: number
}

/** 补齐没有调用的日期；较长的累计用量按月聚合，限制图表节点数。 */
export const createTokenTrend = (
  days: { day: string; totalTokens: number }[],
  from?: number
): { points: StatisticsChartPoint[]; interval: 'day' | 'month' } => {
  if (!days.length && from === undefined) return { points: [], interval: 'day' }
  const first = Math.min(...days.map((day) => new Date(`${day.day}T00:00:00`).getTime()))
  const start = new Date(from ?? first)
  start.setHours(0, 0, 0, 0)
  const interval = Date.now() - start.getTime() > 366 * 86400000 ? 'month' : 'day'
  const key = (day: string): string => (interval === 'month' ? day.slice(0, 7) : day)
  const values = new Map<string, number>()
  for (const day of days)
    values.set(key(day.day), (values.get(key(day.day)) ?? 0) + day.totalTokens)
  if (interval === 'month') start.setDate(1)
  const points: StatisticsChartPoint[] = []
  for (const cursor = start; cursor.getTime() <= Date.now(); ) {
    const dateKey = `${cursor.getFullYear()}-${String(cursor.getMonth() + 1).padStart(2, '0')}-${String(cursor.getDate()).padStart(2, '0')}`
    points.push({ timestamp: cursor.getTime(), value: values.get(key(dateKey)) ?? 0 })
    if (interval === 'month') cursor.setMonth(cursor.getMonth() + 1)
    else cursor.setDate(cursor.getDate() + 1)
  }
  return { points, interval }
}
