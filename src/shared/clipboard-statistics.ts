export const COPY_STATISTICS_TYPES = [
  'text',
  'html',
  'image',
  'rtf',
  'files',
  'url',
  'email',
  'phone',
  'code',
  'color'
] as const
export type CopyStatisticsType = (typeof COPY_STATISTICS_TYPES)[number]
export type StatisticsInterval = 'hour' | 'day' | 'month'

export interface CopyStatisticsEvent {
  timestamp: number
  type: CopyStatisticsType
  source: string
  bytes: number
  files: number
}

export interface CopyStatisticsQuery {
  from?: number
  to?: number // 不包含结束时间；统计精度为小时。
  type?: CopyStatisticsType
  source?: string
  interval?: StatisticsInterval
}

export interface CopyStatisticsTotals {
  copies: number
  bytes: number
  files: number
  firstCopiedAt: number | null
  lastCopiedAt: number | null
}

export interface CopyStatisticsGroup extends CopyStatisticsTotals {
  key: string
}

export interface CopyStatistics extends CopyStatisticsTotals {
  clearedAt: number | null
  activeDays: number
  types: CopyStatisticsGroup[]
  sources: CopyStatisticsGroup[]
  availableSources: string[]
  series: (CopyStatisticsGroup & { timestamp: number })[]
  interval: StatisticsInterval
}
