export type AIOCRProvider = 'openai' | 'claude'

export const AI_OCR_REQUEST_TIMEOUT_MS = 120_000

export interface AIOCRConfig {
  baseUrl: string
  apiKey: string
  model: string
  api?: 'chat' | 'responses'
}

export type AIOCRConfigs = Record<AIOCRProvider, AIOCRConfig>

export interface AIOCRModelOption {
  id: string
  name: string
}

export interface AIOCRAPIRequest {
  provider: AIOCRProvider
  config: AIOCRConfig
}

export interface AIOCRUsageEntry {
  id: string
  provider: AIOCRProvider
  baseUrl: string
  model: string
  purpose: 'ocr' | 'connection-test'
  startedAt: number
  finishedAt: number
  durationMs: number
  success: boolean
  inputTokens: number | null
  outputTokens: number | null
  cachedInputTokens: number
  cacheWriteTokens: number
  totalTokens: number | null
}

export interface AIOCRUsageTotals {
  requests: number
  failures: number
  unknownUsage: number
  inputTokens: number
  outputTokens: number
  cachedInputTokens: number
  cacheWriteTokens: number
  totalTokens: number
  durationMs: number
  firstUsedAt: number | null
  lastUsedAt: number | null
}

/** 时间范围按本地小时对齐，to 所在小时包含在结果中。 */
export interface AIOCRUsageQuery {
  from?: number
  to?: number
}

export interface AIOCRUsageStats {
  clearedAt: number | null
  totals: AIOCRUsageTotals
  models: (AIOCRUsageTotals & { provider: AIOCRProvider; baseUrl: string; model: string })[]
  days: (AIOCRUsageTotals & { day: string })[]
  recent: AIOCRUsageEntry[]
}

export interface OCRTaskStatus {
  contentId: string
  state: 'queued' | 'running' | 'completed' | 'error' | 'cancelled'
  error?: string
}

export function createDefaultAIOCRConfigs(): AIOCRConfigs {
  return {
    openai: { baseUrl: 'https://api.openai.com/v1', apiKey: '', model: '', api: 'chat' },
    claude: { baseUrl: 'https://api.anthropic.com', apiKey: '', model: '', api: 'chat' }
  }
}

export function isAIOCRProvider(value: unknown): value is AIOCRProvider {
  return value === 'openai' || value === 'claude'
}

export function normalizeAIOCRBaseURL(provider: AIOCRProvider, value: string): string {
  let url: URL
  try {
    url = new URL(value.trim())
  } catch {
    throw new Error('AI OCR Base URL 格式无效')
  }
  if (!['http:', 'https:'].includes(url.protocol))
    throw new Error('AI OCR Base URL 必须使用 HTTP 或 HTTPS')
  if (url.username || url.password || url.search || url.hash)
    throw new Error('AI OCR Base URL 不能包含用户名、密码、查询参数或片段')
  let pathname = url.pathname.replace(/\/+$/, '')
  // OpenAI SDK 相对于 Base URL 调用，Claude SDK 自带 /v1/messages 路径。
  if (provider === 'openai' && !pathname) pathname = '/v1'
  if (provider === 'claude') pathname = pathname.replace(/\/v1$/, '')
  url.pathname = pathname
  return url.toString().replace(/\/+$/, '')
}

export function validateAIOCRConfig(
  provider: AIOCRProvider,
  config: AIOCRConfig,
  required = true,
  requireModel = true
): void {
  if (config.api !== undefined && config.api !== 'chat' && config.api !== 'responses')
    throw new Error('OpenAI API 类型无效')
  if (config.baseUrl.trim()) normalizeAIOCRBaseURL(provider, config.baseUrl)
  else if (required) throw new Error('AI OCR Base URL 不能为空')
  if (required && !config.apiKey.trim()) throw new Error('AI OCR API Key 不能为空')
  if (required && requireModel && !config.model.trim()) throw new Error('AI OCR 模型名称不能为空')
}

export function parseAIOCRAPIRequest(value: unknown, requireModel = true): AIOCRAPIRequest {
  if (!value || typeof value !== 'object') throw new Error('AI OCR 请求配置无效')
  const { provider, config } = value as Record<string, unknown>
  if (!isAIOCRProvider(provider) || !config || typeof config !== 'object')
    throw new Error('AI OCR 请求配置无效')
  const fields = config as Record<string, unknown>
  if (['baseUrl', 'apiKey', 'model'].some((key) => typeof fields[key] !== 'string'))
    throw new Error('AI OCR 请求配置字段无效')
  if (fields.api !== undefined && fields.api !== 'chat' && fields.api !== 'responses')
    throw new Error('OpenAI API 类型无效')
  const parsed = {
    baseUrl: (fields.baseUrl as string).trim(),
    apiKey: (fields.apiKey as string).trim(),
    model: (fields.model as string).trim(),
    api: fields.api === 'responses' ? ('responses' as const) : ('chat' as const)
  }
  validateAIOCRConfig(provider, parsed, true, requireModel)
  return { provider, config: parsed }
}
