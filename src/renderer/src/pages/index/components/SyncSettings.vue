<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useSyncStore } from '@renderer/stores/sync'
import { SETTINGS_LABELS } from '@renderer/constants/desktop-ui'
import { invoke, onSimple } from '@renderer/api/ipc'
import type { ScheduledSyncStatus, SyncResult } from '@main/sync/types'

// 使用同步 store
const syncStore = useSyncStore()

// 本地表单状态（不直接绑定到 store）
const localConfig = ref({
  enabled: false,
  provider: '',
  interval: 30, // 分钟
  direction: 'bidirectional',
  params: {
    webdav: {
      serverUrl: '',
      username: '',
      password: '',
      remotePath: '',
      timeout: 30000,
      useHttps: false
    }
  }
})

// 本地状态
const isTestingConnection = ref(false)
const isSaving = ref(false)
const isManualSyncing = ref(false)
const connectionTestResult = ref<{ success: boolean; message: string } | null>(null)
const saveMessage = ref('')
const syncStatus = ref<ScheduledSyncStatus | null>(null)
const lastSyncTime = ref<string>('')
const hasUnsavedChanges = ref(false)
const currentSyncStatus = ref<string>('idle') // 当前同步状态：idle, syncing, success, error
const syncStatusMessage = ref<string>('') // 同步状态消息
let disposed = false
let statusTimer: ReturnType<typeof setTimeout> | undefined
let saveMessageTimer: ReturnType<typeof setTimeout> | undefined

// 同步间隔选项（分钟）
const intervalOptions = [
  { label: '5分钟', value: 5 },
  { label: '10分钟', value: 10 },
  { label: '15分钟', value: 15 },
  { label: '30分钟', value: 30 },
  { label: '1小时', value: 60 },
  { label: '2小时', value: 120 },
  { label: '6小时', value: 360 },
  { label: '12小时', value: 720 },
  { label: '24小时', value: 1440 }
]

// 同步方向选项
const directionOptions = [
  { label: '双向同步', value: 'bidirectional', description: '本地和远程文件双向同步' },
  { label: '仅上传', value: 'upload', description: '仅将本地文件上传到远程' },
  { label: '仅下载', value: 'download', description: '仅从远程下载文件到本地' }
]

// 初始化本地配置
const initializeLocalConfig = (): void => {
  localConfig.value = {
    enabled: syncStore.enabled,
    provider: syncStore.provider,
    interval: Math.floor(syncStore.interval / 60000), // 从毫秒转换为分钟
    direction: syncStore.direction,
    params: {
      webdav: { ...syncStore.params.webdav }
    }
  }
  hasUnsavedChanges.value = false
}

// 计算属性
const isWebDAVProvider = computed(() => localConfig.value.provider === 'webdav')

// 切换同步启用状态
const toggleSyncEnabled = (): void => {
  localConfig.value.enabled = !localConfig.value.enabled
  hasUnsavedChanges.value = true
  // 清除之前的连接测试结果
  connectionTestResult.value = null
}

// 更新同步提供者
const updateProvider = (provider: string): void => {
  localConfig.value.provider = provider
  hasUnsavedChanges.value = true
  // 清除之前的连接测试结果
  connectionTestResult.value = null
}

// 更新同步方向
const updateDirection = (direction: string): void => {
  localConfig.value.direction = direction
  hasUnsavedChanges.value = true
}

// 更新同步间隔
const updateInterval = (interval: number): void => {
  localConfig.value.interval = interval
  hasUnsavedChanges.value = true
}

// 更新 WebDAV 配置
type WebDAVFields = ReturnType<typeof useSyncStore>['params']['webdav']
const updateWebDAVConfig = <K extends keyof WebDAVFields>(
  field: K,
  value: WebDAVFields[K]
): void => {
  if (localConfig.value.params.webdav) {
    localConfig.value.params.webdav[field] = value
    hasUnsavedChanges.value = true
    // 清除之前的连接测试结果
    connectionTestResult.value = null
  }
}

// 验证 URL 格式
const isValidUrl = (url: string): boolean => {
  try {
    new URL(url)
    return true
  } catch {
    return false
  }
}

// 验证配置完整性
const isConfigValid = computed(() => {
  if (!localConfig.value.enabled) return true
  if (!isWebDAVProvider.value) return false

  const webdav = localConfig.value.params.webdav
  return !!(webdav.serverUrl && isValidUrl(webdav.serverUrl) && webdav.username && webdav.password)
})

/**
 * 构建同步配置对象
 * 用于测试连接和保存配置时的统一配置构建
 */
const buildSyncConfig = (): {
  enabled: boolean
  syncInterval: number
  providerType: 'webdav'
  providerConfig: {
    syncDirection: string
    serverUrl: string
    username: string
    password: string
    remotePath: string
    timeout: number
    useHttps: boolean
  }
} => {
  if (!isWebDAVProvider.value) {
    throw new Error('当前仅支持 WebDAV 提供者')
  }

  const webdav = localConfig.value.params.webdav

  return {
    enabled: true, // 测试时总是启用
    syncInterval: localConfig.value.interval,
    providerType: 'webdav' as const,
    providerConfig: {
      syncDirection: localConfig.value.direction,
      serverUrl: webdav.serverUrl.trim(),
      username: webdav.username.trim(),
      password: webdav.password,
      remotePath: webdav.remotePath?.trim() || '/',
      timeout: webdav.timeout || 30000,
      useHttps: webdav.serverUrl?.trim().startsWith('https') || false
    }
  }
}

// 测试连接（使用传入配置的方式）
const testConnection = async (): Promise<void> => {
  // 预检查
  if (!isWebDAVProvider.value) {
    connectionTestResult.value = { success: false, message: '请先选择同步提供者' }
    return
  }

  if (!isConfigValid.value) {
    connectionTestResult.value = {
      success: false,
      message: '请填写完整的服务器信息并确保 URL 格式正确'
    }
    return
  }

  isTestingConnection.value = true
  connectionTestResult.value = null

  try {
    // 构建测试配置并直接传入进行测试
    const testConfig = buildSyncConfig()

    // 调用优化后的测试连接 API，直接传入配置
    const result = await invoke('scheduled-sync:test-connection', testConfig)

    connectionTestResult.value = {
      success: result === true,
      message:
        result === true
          ? '连接测试成功！服务器配置正确'
          : '连接失败，请检查服务器地址、用户名和密码是否正确'
    }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : '未知错误'
    connectionTestResult.value = {
      success: false,
      message: `连接测试失败: ${errorMessage}`
    }
  } finally {
    isTestingConnection.value = false
  }
}

// 显示保存消息
const showSaveMessage = (message: string): void => {
  if (disposed) return
  saveMessage.value = message
  if (saveMessageTimer) clearTimeout(saveMessageTimer)
  saveMessageTimer = setTimeout(() => {
    saveMessage.value = ''
  }, 3000)
}

// 保存配置
const saveConfig = async (): Promise<void> => {
  if (isSaving.value) return
  isSaving.value = true
  connectionTestResult.value = null

  try {
    // 如果启用同步，先测试连通性
    if (localConfig.value.enabled) {
      if (!isConfigValid.value) {
        showSaveMessage('配置不完整，请填写完整的服务器信息并确保 URL 格式正确')
        return
      }

      // 测试连通性
      showSaveMessage('正在测试连接...')

      try {
        // 使用统一的配置构建函数并直接传入进行测试
        const testConfig = buildSyncConfig()
        const connectionResult = await invoke('scheduled-sync:test-connection', testConfig)

        if (connectionResult !== true) {
          showSaveMessage('连接测试失败，请检查服务器地址、用户名和密码是否正确')
          return
        }
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : '未知错误'
        showSaveMessage(`连接测试失败: ${errorMessage}`)
        return
      }
    }

    // 保存到 store
    syncStore.$patch({
      enabled: localConfig.value.enabled,
      provider: localConfig.value.provider,
      interval: localConfig.value.interval * 60000,
      direction: localConfig.value.direction,
      params: { webdav: { ...localConfig.value.params.webdav } }
    })
    await syncStore.$whenSynced()
    initializeLocalConfig()
    showSaveMessage('配置保存成功')

    // 刷新同步状态
    await getSyncStatus()
  } catch (error) {
    showSaveMessage('保存失败: ' + (error as Error).message)
  } finally {
    isSaving.value = false
  }
}

// 重置配置
const resetConfig = (): void => {
  initializeLocalConfig()
  connectionTestResult.value = null
  showSaveMessage('配置已重置')
}

// 手动同步
const manualSync = async (): Promise<void> => {
  // 防止重复点击
  if (isManualSyncing.value) return

  // 检查配置是否有效，而不是检查同步是否启用
  if (!isWebDAVProvider.value) {
    alert('请先选择同步提供者')
    return
  }

  // 使用本地配置检查，因为可能还没有保存到 store
  const webdavConfig = localConfig.value.params.webdav
  if (!webdavConfig.serverUrl || !webdavConfig.username || !webdavConfig.password) {
    alert('请先配置完整的同步服务器信息')
    return
  }

  if (!isValidUrl(webdavConfig.serverUrl)) {
    alert('请输入有效的服务器地址')
    return
  }

  isManualSyncing.value = true

  try {
    let result: { success: boolean; message?: string; error?: string }
    if (syncStore.enabled) {
      // 如果同步已启用，直接使用现有配置
      result = await invoke('scheduled-sync:manual-sync')
    } else {
      // 如果同步未启用，构建临时配置用于手动同步
      const syncConfig = buildSyncConfig()
      result = await invoke('scheduled-sync:manual-sync-with-config', syncConfig)
    }

    if (result.success) {
      alert('手动同步完成: ' + result.message)
      // 刷新同步状态
      await getSyncStatus()
    } else {
      alert('手动同步失败: ' + (result.message || result.error || '未知错误'))
    }
  } catch (error) {
    alert('手动同步失败: ' + (error as Error).message)
  } finally {
    isManualSyncing.value = false
  }
}

// 获取同步状态
const getSyncStatus = async (): Promise<void> => {
  try {
    const result = await invoke('scheduled-sync:get-status')
    if (!disposed && result.success && result.data) {
      syncStatus.value = result.data
      if (result.data.lastSyncTime) {
        lastSyncTime.value = new Date(result.data.lastSyncTime).toLocaleString()
      }
    }
  } catch (error) {
    console.error('获取同步状态失败:', error)
  }
}

// IPC事件监听器清理函数
let syncStatusUnsubscribe: (() => void) | null = null

// 处理同步状态更新事件
const handleSyncStatusUpdate = (data: {
  type: string
  timestamp?: number
  result?: SyncResult
}): void => {
  if (statusTimer) clearTimeout(statusTimer)
  switch (data.type) {
    case 'sync-started':
    case 'manual-sync-started':
      currentSyncStatus.value = 'syncing'
      syncStatusMessage.value =
        data.type === 'sync-started' ? '正在执行定时同步...' : '正在执行手动同步...'
      break

    case 'sync-completed':
    case 'manual-sync-completed':
      currentSyncStatus.value = 'success'
      syncStatusMessage.value = data.result?.message || '同步完成'
      // 更新最后同步时间
      if (data.timestamp) {
        lastSyncTime.value = new Date(data.timestamp).toLocaleString()
      }
      // 3秒后清除状态消息
      statusTimer = setTimeout(() => {
        if (currentSyncStatus.value === 'success') {
          currentSyncStatus.value = 'idle'
          syncStatusMessage.value = ''
        }
      }, 3000)
      break

    case 'sync-failed':
    case 'manual-sync-failed':
      currentSyncStatus.value = 'error'
      syncStatusMessage.value = data.result?.message || '同步失败'
      // 5秒后清除错误状态
      statusTimer = setTimeout(() => {
        if (currentSyncStatus.value === 'error') {
          currentSyncStatus.value = 'idle'
          syncStatusMessage.value = ''
        }
      }, 5000)
      break
  }

  // 刷新同步状态
  getSyncStatus()
}

watch(
  () => syncStore.$state,
  () => {
    if (!isSaving.value && !hasUnsavedChanges.value) initializeLocalConfig()
  },
  { deep: true }
)

onMounted(() => {
  // 初始化本地配置
  initializeLocalConfig()

  syncStatusUnsubscribe = onSimple('sync-status-updated', handleSyncStatusUpdate)
  void getSyncStatus()
})

onUnmounted(() => {
  disposed = true
  if (statusTimer) clearTimeout(statusTimer)
  if (saveMessageTimer) clearTimeout(saveMessageTimer)
  // 清理IPC事件监听器
  if (syncStatusUnsubscribe) {
    syncStatusUnsubscribe()
    syncStatusUnsubscribe = null
  }
})
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">同步设置</h3>
      <p class="section-subtitle">{{ SETTINGS_LABELS.configSyncDescription }}</p>
    </div>

    <!-- 保存提示消息 -->
    <div v-if="saveMessage" class="save-message">
      {{ saveMessage }}
    </div>

    <!-- 未保存更改提示 -->
    <div v-if="hasUnsavedChanges" class="unsaved-changes-warning">
      {{ SETTINGS_LABELS.unsavedChanges }}
    </div>

    <div class="settings-grid">
      <!-- 启用同步 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">启用同步</h4>
          <p class="setting-description">开启后将自动同步剪贴板历史记录到云端</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input type="checkbox" :checked="localConfig.enabled" @change="toggleSyncEnabled" />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- 同步提供者 -->
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">同步提供者</h4>
          <p class="setting-description">选择用于同步的云存储服务</p>
        </div>
        <div class="setting-control">
          <select
            :value="localConfig.provider"
            class="select-input"
            @change="updateProvider(($event.target as HTMLSelectElement).value)"
          >
            <option value="">请选择同步提供者</option>
            <option value="webdav">WebDAV</option>
          </select>
        </div>
      </div>

      <!-- 同步间隔 -->
      <div v-if="localConfig.enabled" class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">同步间隔</h4>
          <p class="setting-description">设置自动同步的时间间隔</p>
        </div>
        <div class="setting-control">
          <select
            :value="localConfig.interval"
            class="select-input"
            @change="updateInterval(parseInt(($event.target as HTMLSelectElement).value))"
          >
            <option v-for="option in intervalOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
      </div>

      <!-- 同步方向 -->
      <div v-if="localConfig.enabled" class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">同步方向</h4>
          <p class="setting-description">{{ SETTINGS_LABELS.configSyncDirection }}</p>
        </div>
        <div class="setting-control">
          <div class="radio-group">
            <label v-for="option in directionOptions" :key="option.value" class="radio-option">
              <input
                type="radio"
                :value="option.value"
                :checked="localConfig.direction === option.value"
                class="radio-input"
                @change="updateDirection(option.value)"
              />
              <span class="radio-label">
                <strong>{{ option.label }}</strong>
                <span class="radio-description">{{ option.description }}</span>
              </span>
            </label>
          </div>
        </div>
      </div>
    </div>

    <!-- WebDAV 配置 -->
    <div v-if="isWebDAVProvider" class="webdav-config">
      <div class="section-header">
        <h4 class="section-title">WebDAV 服务器配置</h4>
        <p class="section-subtitle">配置 WebDAV 服务器连接信息</p>
      </div>

      <div class="settings-grid">
        <!-- 服务器地址 -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">服务器地址</h4>
            <p class="setting-description">WebDAV 服务器的完整 URL 地址</p>
          </div>
          <div class="setting-control">
            <input
              type="url"
              :value="localConfig.params.webdav.serverUrl"
              placeholder="https://example.com/webdav"
              class="text-input"
              @input="updateWebDAVConfig('serverUrl', ($event.target as HTMLInputElement).value)"
            />
          </div>
        </div>

        <!-- 用户名 -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">用户名</h4>
            <p class="setting-description">WebDAV 服务器登录用户名</p>
          </div>
          <div class="setting-control">
            <input
              type="text"
              :value="localConfig.params.webdav.username"
              placeholder="输入用户名"
              class="text-input"
              @input="updateWebDAVConfig('username', ($event.target as HTMLInputElement).value)"
            />
          </div>
        </div>

        <!-- 密码 -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">密码</h4>
            <p class="setting-description">WebDAV 服务器登录密码</p>
          </div>
          <div class="setting-control">
            <input
              type="password"
              :value="localConfig.params.webdav.password"
              placeholder="输入密码"
              class="text-input"
              @input="updateWebDAVConfig('password', ($event.target as HTMLInputElement).value)"
            />
          </div>
        </div>

        <!-- 远程路径 -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">远程路径</h4>
            <p class="setting-description">存储同步文件的远程目录路径</p>
          </div>
          <div class="setting-control">
            <input
              type="text"
              :value="localConfig.params.webdav.remotePath"
              placeholder="/super-paste"
              class="text-input"
              @input="updateWebDAVConfig('remotePath', ($event.target as HTMLInputElement).value)"
            />
          </div>
        </div>

        <!-- 超时时间 -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">超时时间</h4>
            <p class="setting-description">网络请求超时时间（毫秒）</p>
          </div>
          <div class="setting-control">
            <div class="number-input-container">
              <input
                type="number"
                :value="localConfig.params.webdav.timeout"
                min="5000"
                max="120000"
                step="1000"
                class="number-input"
                @input="
                  updateWebDAVConfig(
                    'timeout',
                    parseInt(($event.target as HTMLInputElement).value) || 30000
                  )
                "
              />
              <span class="number-unit">毫秒</span>
            </div>
          </div>
        </div>

        <!-- 使用 HTTPS -->
        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">使用 HTTPS</h4>
            <p class="setting-description">启用 HTTPS 安全连接</p>
          </div>
          <div class="setting-control">
            <label class="toggle-switch">
              <input
                type="checkbox"
                :checked="localConfig.params.webdav.useHttps"
                @change="
                  updateWebDAVConfig('useHttps', ($event.target as HTMLInputElement).checked)
                "
              />
              <span class="toggle-slider"></span>
            </label>
          </div>
        </div>
      </div>

      <!-- 连接测试和操作按钮 -->
      <div class="sync-actions">
        <div class="action-buttons">
          <button
            :disabled="isTestingConnection || !isConfigValid"
            class="shortcut-button secondary"
            @click="testConnection"
          >
            {{ isTestingConnection ? '测试中...' : '测试连接' }}
          </button>

          <button
            :disabled="!isConfigValid || isManualSyncing"
            class="shortcut-button primary"
            @click="manualSync"
          >
            {{ isManualSyncing ? '同步中...' : '手动同步' }}
          </button>

          <button class="shortcut-button secondary" @click="getSyncStatus">刷新状态</button>
        </div>

        <!-- 配置验证提示 -->
        <div v-if="!isConfigValid" class="config-warning">
          配置不完整，请填写完整的服务器信息并确保 URL 格式正确
        </div>

        <!-- 连接测试结果 -->
        <div
          v-if="connectionTestResult"
          class="connection-result"
          :class="{ success: connectionTestResult.success, error: !connectionTestResult.success }"
        >
          {{ connectionTestResult.message }}
        </div>
      </div>
    </div>

    <!-- 配置保存区域 -->
    <div class="config-save-section">
      <div class="section-header">
        <h4 class="section-title">配置管理</h4>
        <p class="section-subtitle">保存或重置同步配置</p>
      </div>

      <div class="save-actions">
        <button
          :disabled="isSaving || (!hasUnsavedChanges && !localConfig.enabled)"
          class="shortcut-button primary save-button"
          @click="saveConfig"
        >
          {{ isSaving ? '保存中...' : '保存配置' }}
        </button>

        <button
          :disabled="isSaving || !hasUnsavedChanges"
          class="shortcut-button secondary"
          @click="resetConfig"
        >
          重置配置
        </button>
      </div>
    </div>

    <!-- 同步状态信息 -->
    <div v-if="syncStore.enabled" class="sync-status">
      <div class="section-header">
        <h4 class="section-title">同步状态</h4>
        <p class="section-subtitle">查看当前同步状态和历史记录</p>
      </div>

      <div class="status-grid">
        <div class="status-card">
          <div class="status-info">
            <h5 class="status-title">同步状态</h5>
            <p
              class="status-value"
              :class="{
                'status-enabled': syncStore.enabled,
                'status-disabled': !syncStore.enabled
              }"
            >
              {{ syncStore.enabled ? '已启用' : '已禁用' }}
            </p>
          </div>
        </div>

        <!-- 实时同步状态 -->
        <div v-if="currentSyncStatus !== 'idle' || syncStatusMessage" class="status-card">
          <div class="status-info">
            <h5 class="status-title">实时状态</h5>
            <p
              class="status-value"
              :class="{
                'status-syncing': currentSyncStatus === 'syncing',
                'status-success': currentSyncStatus === 'success',
                'status-error': currentSyncStatus === 'error'
              }"
            >
              <span v-if="currentSyncStatus === 'syncing'" class="sync-spinner">⟳</span>
              {{ syncStatusMessage || '空闲' }}
            </p>
          </div>
        </div>

        <div class="status-card">
          <div class="status-info">
            <h5 class="status-title">同步间隔</h5>
            <p class="status-value">
              {{
                intervalOptions.find((opt) => opt.value === Math.floor(syncStore.interval / 60000))
                  ?.label || '未设置'
              }}
            </p>
          </div>
        </div>

        <div class="status-card">
          <div class="status-info">
            <h5 class="status-title">同步方向</h5>
            <p class="status-value">
              {{
                directionOptions.find((opt) => opt.value === syncStore.direction)?.label || '未设置'
              }}
            </p>
          </div>
        </div>

        <div class="status-card">
          <div class="status-info">
            <h5 class="status-title">最后同步</h5>
            <p class="status-value">{{ lastSyncTime || '暂无记录' }}</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 保存消息样式 */
.save-message {
  background: var(--color-success-bg);
  border: 1px solid var(--color-success-border);
  border-radius: var(--radius-sm);
  padding: 12px 16px;
  font-size: 13px;
  color: var(--color-success-text);
  margin-bottom: 20px;
  font-weight: 500;
}

/* 未保存更改警告 */
.unsaved-changes-warning {
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  border-radius: var(--radius-sm);
  padding: 12px 16px;
  font-size: 13px;
  color: var(--color-warning-text);
  margin-bottom: 20px;
  font-weight: 500;
}

/* 配置保存区域 */
.config-save-section {
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid var(--color-border-light);
}

.save-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 16px;
}

.save-button {
  min-width: 96px;
}

/* WebDAV 配置区域 */
.webdav-config {
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid var(--color-border-light);
}

.webdav-config .setting-control:has(.text-input) {
  width: clamp(200px, 45%, 360px);
}

/* 选择框样式 */
.select-input {
  width: 100%;
  padding: 5px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  font-size: 13px;
  background: var(--color-bg-primary);
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
}

.select-input:focus {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: var(--shadow-focus-primary);
}

/* 文本输入框样式 */
.text-input {
  width: 100%;
  padding: 5px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  font-size: 13px;
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
}

.text-input:focus {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: var(--shadow-focus-primary);
}

/* 单选按钮组样式 */
.radio-group {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.radio-option {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  cursor: pointer;
  padding: 6px;
  border-radius: var(--radius-xs);
  transition:
    background-color var(--duration-normal),
    box-shadow var(--duration-normal);
}

.radio-option:hover {
  background: var(--color-selection);
}

.radio-option:has(.radio-input:checked) {
  background: var(--color-selection);
  box-shadow: inset 0 0 0 1px var(--color-primary);
}

.radio-option:has(.radio-input:checked):hover,
.radio-option:active {
  background: var(--color-selection-hover);
}

.radio-input {
  flex-shrink: 0;
  accent-color: var(--color-primary);
  margin-top: 2px;
}

.radio-label {
  display: flex;
  min-width: 0;
  overflow-wrap: anywhere;
  flex-direction: column;
  gap: 4px;
}

.radio-description {
  font-size: 12px;
  color: var(--color-text-secondary);
  font-weight: normal;
}

/* 同步操作按钮区域 */
.sync-actions {
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid var(--color-border-light);
}

.action-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 16px;
}

/* 配置警告 */
.config-warning {
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  border-radius: var(--radius-xs);
  padding: 12px 16px;
  font-size: 13px;
  color: var(--color-warning-text);
  font-weight: 500;
  margin-bottom: 12px;
}

/* 连接测试结果 */
.connection-result {
  padding: 12px 16px;
  border-radius: var(--radius-xs);
  font-size: 13px;
  font-weight: 500;
}

.connection-result.success {
  background: var(--color-success-bg);
  border: 1px solid var(--color-success-border);
  color: var(--color-success-text);
}

.connection-result.error {
  background: var(--color-danger-bg);
  border: 1px solid var(--color-danger-border);
  color: var(--color-danger);
}

/* 同步状态区域 */
.sync-status {
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid var(--color-border-light);
}

.status-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(200px, 100%), 1fr));
  gap: 8px;
  margin-top: 16px;
}

.status-card {
  min-width: 0;
  overflow-wrap: anywhere;
  background: var(--color-bg-primary);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm);
  padding: 16px;
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
}

.status-info {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.status-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin: 0;
}

.status-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text-primary);
  margin: 0;
}

.status-value.status-enabled {
  color: var(--color-success-text);
}

.status-value.status-disabled {
  color: var(--color-danger);
}

.status-value.status-syncing {
  color: var(--color-primary);
}

.status-value.status-success {
  color: var(--color-success-text);
}

.status-value.status-error {
  color: var(--color-danger);
}

/* 同步中的旋转动画 */
.sync-spinner {
  display: inline-block;
  animation: spin 1s linear infinite;
  margin-right: 4px;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}
</style>
