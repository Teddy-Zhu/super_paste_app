import Store from 'electron-store'
import { safeStorage } from 'electron'
import { createDefaultMCPSettings, type MCPSettings } from '../../shared/mcp'

interface StoredMCPSettings extends Omit<MCPSettings, 'apiKey'> {
  encryptedKey: string
}

let configStore: Store<{ config: StoredMCPSettings }> | undefined

function getStore(): Store<{ config: StoredMCPSettings }> {
  // 独立于通用设置，避免同步配置时复制本机服务开关和密钥。
  return (configStore ??= new Store({ name: 'mcp' }))
}

export function parseMCPSettings(value: unknown): MCPSettings {
  if (!value || typeof value !== 'object') throw new Error('MCP 配置无效')
  const input = value as Record<string, unknown>
  if (typeof input.enabled !== 'boolean' || typeof input.authEnabled !== 'boolean')
    throw new Error('MCP 开关配置无效')
  if (!Number.isInteger(input.port) || Number(input.port) < 1024 || Number(input.port) > 65535)
    throw new Error('MCP 端口须为 1024–65535 的整数')
  if (typeof input.apiKey !== 'string') throw new Error('API Key 无效')
  if (input.apiKey && !/^[\x21-\x7e]{8,256}$/.test(input.apiKey))
    throw new Error('API Key 须为 8–256 位可见 ASCII 字符，且不含空格')
  if (input.enabled && input.authEnabled && !input.apiKey) throw new Error('请先设置或生成 API Key')
  return {
    enabled: input.enabled,
    port: Number(input.port),
    authEnabled: input.authEnabled,
    apiKey: input.apiKey
  }
}

export function loadMCPSettings(): MCPSettings {
  const stored = getStore().get('config')
  if (!stored) return createDefaultMCPSettings()
  if (stored.encryptedKey && !safeStorage.isEncryptionAvailable())
    throw new Error('系统密钥存储不可用，无法解密 MCP API Key')
  const apiKey = stored.encryptedKey
    ? safeStorage.decryptString(Buffer.from(stored.encryptedKey, 'base64'))
    : ''
  return parseMCPSettings({ ...stored, apiKey })
}

export function persistMCPSettings(settings: MCPSettings): void {
  if (settings.apiKey && !safeStorage.isEncryptionAvailable())
    throw new Error('系统密钥存储不可用，无法保存 MCP API Key')
  const { apiKey, ...config } = settings
  getStore().set('config', {
    ...config,
    encryptedKey: apiKey ? safeStorage.encryptString(apiKey).toString('base64') : ''
  })
}
