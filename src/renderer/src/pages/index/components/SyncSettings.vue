<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useSyncStore } from '@renderer/stores/sync'
import { invoke, onSimple } from '@renderer/api/ipc'

// 使用同步 store
const syncStore = useSyncStore()

// 本地表单状态（不直接绑定到 store）
const localConfig = ref({
  enabled: false,
  provider: '',
  interval: 30, // 分钟
  direction: 'bidirectional',
  configOnly: false,
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
const syncStatus = ref<any>(null)
const lastSyncTime = ref<string>('')
const hasUnsavedChanges = ref(false)
const currentSyncStatus = ref<string>('idle') // 当前同步状态：idle, syncing, success, error
const syncStatusMessage = ref<string>('') // 同步状态消息

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
const initializeLocalConfig = () => {
  localConfig.value = {
    enabled: syncStore.enabled,
    provider: syncStore.provider,
    interval: Math.floor(syncStore.interval / 60000), // 从毫秒转换为分钟
    direction: syncStore.direction,
    configOnly: syncStore.configOnly,
    params: {
      webdav: { ...syncStore.params.webdav }
    }
  }
  hasUnsavedChanges.value = false
}

// 计算属性
const isWebDAVProvider = computed(() => localConfig.value.provider === 'webdav')

// 切换同步启用状态
const toggleSyncEnabled = () => {
  localConfig.value.enabled = !localConfig.value.enabled
  hasUnsavedChanges.value = true
  // 清除之前的连接测试结果
  connectionTestResult.value = null
}

// 更新同步提供者
const updateProvider = (provider: string) => {
  localConfig.value.provider = provider
  hasUnsavedChanges.value = true
  // 清除之前的连接测试结果
  connectionTestResult.value = null
}

// 更新同步方向
const updateDirection = (direction: string) => {
  localConfig.value.direction = direction
  hasUnsavedChanges.value = true
}

// 更新同步间隔
const updateInterval = (interval: number) => {
  localConfig.value.interval = interval
  hasUnsavedChanges.value = true
}

// 更新仅同步配置选项
const updateConfigOnly = (configOnly: boolean) => {
  localConfig.value.configOnly = configOnly
  hasUnsavedChanges.value = true
}

// 更新 WebDAV 配置
const updateWebDAVConfig = (field: string, value: any) => {
  if (localConfig.value.params.webdav) {
    ;(localConfig.value.params.webdav as any)[field] = value
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
const buildSyncConfig = () => {
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
      configOnly: localConfig.value.configOnly,
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
const testConnection = async () => {
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
      success: result,
      message: result
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
const showSaveMessage = (message: string) => {
  saveMessage.value = message
  setTimeout(() => {
    saveMessage.value = ''
  }, 3000)
}

// 保存配置
const saveConfig = async () => {
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

        if (!connectionResult) {
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
    syncStore.enabled = localConfig.value.enabled
    syncStore.provider = localConfig.value.provider
    syncStore.interval = localConfig.value.interval * 60000 // 转换为毫秒
    syncStore.direction = localConfig.value.direction
    syncStore.configOnly = localConfig.value.configOnly
    syncStore.params.webdav = { ...localConfig.value.params.webdav }

    // 等待1秒后刷新计划同步任务
    await new Promise((resolve) => setTimeout(resolve, 1000))
    await invoke('scheduled-sync:refresh')

    hasUnsavedChanges.value = false
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
const resetConfig = () => {
  initializeLocalConfig()
  connectionTestResult.value = null
  showSaveMessage('配置已重置')
}

// 手动同步
const manualSync = async () => {
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
    let result: any
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
      alert('手动同步失败: ' + result.message)
    }
  } catch (error) {
    alert('手动同步失败: ' + (error as Error).message)
  } finally {
    isManualSyncing.value = false
  }
}

// 获取同步状态
const getSyncStatus = async () => {
  try {
    const result = await invoke('scheduled-sync:get-status')
    if (result.success) {
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
const handleSyncStatusUpdate = (data: any) => {
  console.log('收到同步状态更新:', data)

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
      setTimeout(() => {
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
      setTimeout(() => {
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

onMounted(async () => {
  // 组件挂载时的初始化逻辑
  console.log('同步设置组件已挂载', syncStore)

  // 初始化本地配置
  initializeLocalConfig()

  // 获取当前同步状态
  await getSyncStatus()

  // 设置IPC事件监听器
  syncStatusUnsubscribe = onSimple('sync-status-updated', handleSyncStatusUpdate)
})

onUnmounted(() => {
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
      <p class="section-subtitle">配置数据库和配置文件的云端同步</p>
    </div>

    <!-- 保存提示消息 -->
    <div v-if="saveMessage" class="save-message">
      {{ saveMessage }}
    </div>

    <!-- 未保存更改提示 -->
    <div v-if="hasUnsavedChanges" class="unsaved-changes-warning">
      ⚠️ 您有未保存的更改，请点击保存按钮保存配置
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
            @change="updateProvider(($event.target as HTMLSelectElement).value)"
            class="select-input"
          >
            <option value="">请选择同步提供者</option>
            <option value="webdav">WebDAV</option>
          </select>
        </div>
      </div>

      <!-- 同步间隔 -->
      <div class="setting-card" v-if="localConfig.enabled">
        <div class="setting-info">
          <h4 class="setting-title">同步间隔</h4>
          <p class="setting-description">设置自动同步的时间间隔</p>
        </div>
        <div class="setting-control">
          <select
            :value="localConfig.interval"
            @change="updateInterval(parseInt(($event.target as HTMLSelectElement).value))"
            class="select-input"
          >
            <option v-for="option in intervalOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
      </div>

      <!-- 仅同步配置 -->
      <div class="setting-card" v-if="localConfig.enabled">
        <div class="setting-info">
          <h4 class="setting-title">仅同步配置</h4>
          <p class="setting-description">仅同步配置文件，不进行数据同步</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input
              type="checkbox"
              :checked="localConfig.configOnly"
              @change="updateConfigOnly(($event.target as HTMLInputElement).checked)"
            />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- 同步方向 -->
      <div class="setting-card" v-if="localConfig.enabled">
        <div class="setting-info">
          <h4 class="setting-title">同步方向</h4>
          <p class="setting-description">选择数据同步的方向</p>
        </div>
        <div class="setting-control">
          <div class="radio-group">
            <label v-for="option in directionOptions" :key="option.value" class="radio-option">
              <input
                type="radio"
                :value="option.value"
                :checked="localConfig.direction === option.value"
                @change="updateDirection(option.value)"
                class="radio-input"
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
              @input="updateWebDAVConfig('serverUrl', ($event.target as HTMLInputElement).value)"
              placeholder="https://example.com/webdav"
              class="text-input"
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
              @input="updateWebDAVConfig('username', ($event.target as HTMLInputElement).value)"
              placeholder="输入用户名"
              class="text-input"
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
              @input="updateWebDAVConfig('password', ($event.target as HTMLInputElement).value)"
              placeholder="输入密码"
              class="text-input"
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
              @input="updateWebDAVConfig('remotePath', ($event.target as HTMLInputElement).value)"
              placeholder="/super-paste"
              class="text-input"
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
                @input="
                  updateWebDAVConfig(
                    'timeout',
                    parseInt(($event.target as HTMLInputElement).value) || 30000
                  )
                "
                min="5000"
                max="120000"
                step="1000"
                class="number-input"
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
            @click="testConnection"
            :disabled="isTestingConnection || !isConfigValid"
            class="shortcut-button secondary"
          >
            {{ isTestingConnection ? '测试中...' : '测试连接' }}
          </button>

          <button
            @click="manualSync"
            :disabled="!isConfigValid || isManualSyncing"
            class="shortcut-button primary"
          >
            {{ isManualSyncing ? '同步中...' : '手动同步' }}
          </button>

          <button @click="getSyncStatus" class="shortcut-button secondary">刷新状态</button>
        </div>

        <!-- 配置验证提示 -->
        <div v-if="!isConfigValid" class="config-warning">
          ⚠️ 配置不完整，请填写完整的服务器信息并确保 URL 格式正确
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
          @click="saveConfig"
          :disabled="isSaving || (!hasUnsavedChanges && !localConfig.enabled)"
          class="shortcut-button primary save-button"
        >
          {{ isSaving ? '保存中...' : '保存配置' }}
        </button>

        <button
          @click="resetConfig"
          :disabled="isSaving || !hasUnsavedChanges"
          class="shortcut-button secondary"
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
        <div class="status-card" v-if="currentSyncStatus !== 'idle' || syncStatusMessage">
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
  background: linear-gradient(135deg, #e8f5e8 0%, #d4edda 100%);
  border: 1px solid rgba(40, 167, 69, 0.2);
  border-radius: 10px;
  padding: 12px 16px;
  font-size: 13px;
  color: #155724;
  margin-bottom: 20px;
  font-weight: 500;
  box-shadow: 0 2px 8px rgba(40, 167, 69, 0.1);
}

/* 未保存更改警告 */
.unsaved-changes-warning {
  background: linear-gradient(135deg, #fff3cd 0%, #ffeaa7 100%);
  border: 1px solid rgba(255, 193, 7, 0.3);
  border-radius: 10px;
  padding: 12px 16px;
  font-size: 13px;
  color: #856404;
  margin-bottom: 20px;
  font-weight: 500;
  box-shadow: 0 2px 8px rgba(255, 193, 7, 0.1);
}

/* 配置保存区域 */
.config-save-section {
  margin-top: 32px;
  padding-top: 24px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

.save-actions {
  display: flex;
  gap: 16px;
  margin-top: 16px;
}

.save-button {
  min-width: 120px;
}

/* WebDAV 配置区域 */
.webdav-config {
  margin-top: 32px;
  padding-top: 24px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

/* 选择框样式 */
.select-input {
  width: 100%;
  padding: 10px 12px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  font-size: 14px;
  background: white;
  transition: all 0.2s ease;
}

.select-input:focus {
  outline: none;
  border-color: #007aff;
  box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
}

/* 文本输入框样式 */
.text-input {
  width: 100%;
  padding: 10px 12px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  font-size: 14px;
  transition: all 0.2s ease;
}

.text-input:focus {
  outline: none;
  border-color: #007aff;
  box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
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
  padding: 12px;
  border-radius: 8px;
  transition: background-color 0.2s ease;
}

.radio-option:hover {
  background: rgba(0, 122, 255, 0.05);
}

.radio-input {
  margin-top: 2px;
}

.radio-label {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.radio-description {
  font-size: 12px;
  color: #86868b;
  font-weight: normal;
}

/* 同步操作按钮区域 */
.sync-actions {
  margin-top: 24px;
  padding-top: 20px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

.action-buttons {
  display: flex;
  gap: 16px;
  margin-bottom: 16px;
}

/* 配置警告 */
.config-warning {
  background: linear-gradient(135deg, #fff3cd 0%, #ffeaa7 100%);
  border: 1px solid rgba(255, 193, 7, 0.3);
  border-radius: 8px;
  padding: 12px 16px;
  font-size: 13px;
  color: #856404;
  font-weight: 500;
  margin-bottom: 12px;
}

/* 连接测试结果 */
.connection-result {
  padding: 12px 16px;
  border-radius: 8px;
  font-size: 13px;
  font-weight: 500;
}

.connection-result.success {
  background: linear-gradient(135deg, #e8f5e8 0%, #d4edda 100%);
  border: 1px solid rgba(40, 167, 69, 0.2);
  color: #155724;
}

.connection-result.error {
  background: linear-gradient(135deg, #ffebee 0%, #ffcdd2 100%);
  border: 1px solid rgba(255, 59, 48, 0.2);
  color: #d70015;
}

/* 同步状态区域 */
.sync-status {
  margin-top: 32px;
  padding-top: 24px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

.status-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 16px;
  margin-top: 16px;
}

.status-card {
  background: linear-gradient(135deg, #f8f9fa 0%, #e9ecef 100%);
  border: 1px solid rgba(0, 0, 0, 0.06);
  border-radius: 12px;
  padding: 16px;
  transition: all 0.2s ease;
}

.status-card:hover {
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}

.status-info {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.status-title {
  font-size: 13px;
  font-weight: 600;
  color: #86868b;
  margin: 0;
}

.status-value {
  font-size: 15px;
  font-weight: 600;
  color: #1d1d1f;
  margin: 0;
}

.status-value.status-enabled {
  color: #28a745;
}

.status-value.status-disabled {
  color: #dc3545;
}

.status-value.status-syncing {
  color: #007aff;
}

.status-value.status-success {
  color: #28a745;
}

.status-value.status-error {
  color: #dc3545;
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
