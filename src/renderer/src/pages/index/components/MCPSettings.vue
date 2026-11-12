<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import IconCopy from '~icons/mdi/content-copy'
import IconRefresh from '~icons/mdi/refresh'
import { invoke, onSimple } from '@renderer/api/ipc'
import NumberInput from '@renderer/components/NumberInput.vue'
import MCPClientGuide from './MCPClientGuide.vue'
import MCPReference from './MCPReference.vue'
import { createDefaultMCPSettings, getMCPEndpoint, MCP_TEXT, type MCPStatus } from '@shared/mcp'

const form = ref(createDefaultMCPSettings())
const saved = ref(createDefaultMCPSettings())
const status = ref<MCPStatus>({ running: false, endpoint: getMCPEndpoint(saved.value.port) })
const loading = ref(true)
const loaded = ref(false)
const refreshing = ref(false)
const saving = ref(false)
const generating = ref(false)
const showKey = ref(false)
const error = ref('')
const message = ref('')
const busy = computed(() => loading.value || refreshing.value || saving.value || generating.value)
const dirty = computed(() =>
  (Object.keys(form.value) as (keyof typeof form.value)[]).some(
    (key) => form.value[key] !== saved.value[key]
  )
)
const serviceState = computed(() =>
  status.value.error ? 'error' : status.value.running ? 'running' : 'stopped'
)
let unsubscribe: (() => void) | undefined

const updatePort = (value: number | undefined): void => {
  if (value !== undefined) form.value.port = value
}
async function refresh(): Promise<void> {
  if (refreshing.value || saving.value) return
  refreshing.value = true
  error.value = ''
  try {
    const response = await invoke('mcp:get-state')
    if (!response.success || !response.data) throw new Error(response.error || '读取 MCP 配置失败')
    saved.value = { ...response.data.settings }
    status.value = response.data.status
    if (!loaded.value) form.value = { ...response.data.settings }
    loaded.value = true
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
    refreshing.value = false
  }
}
async function save(): Promise<void> {
  if (busy.value || !loaded.value || (!dirty.value && !status.value.error)) return
  saving.value = true
  error.value = ''
  message.value = ''
  try {
    const response = await invoke('mcp:save-settings', { ...form.value })
    if (!response.success || !response.data) throw new Error(response.error || '保存 MCP 配置失败')
    saved.value = { ...response.data.settings }
    form.value = { ...response.data.settings }
    status.value = response.data.status
    message.value = MCP_TEXT.saved
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    saving.value = false
  }
}
async function generateKey(): Promise<void> {
  if (busy.value) return
  generating.value = true
  error.value = ''
  message.value = ''
  try {
    const response = await invoke('mcp:generate-key')
    if (!response.success || !response.data) throw new Error(response.error || '生成密钥失败')
    form.value.apiKey = response.data
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    generating.value = false
  }
}
async function copy(text: string): Promise<boolean> {
  error.value = ''
  message.value = ''
  try {
    await navigator.clipboard.writeText(text)
    message.value = MCP_TEXT.copied
    return true
  } catch {
    error.value = MCP_TEXT.copyFailed
    return false
  }
}
onMounted(() => {
  unsubscribe = onSimple('mcp:status', (value: MCPStatus) => {
    status.value = value
  })
  void refresh()
})
onUnmounted(() => unsubscribe?.())
</script>

<template>
  <section class="content-section mcp-settings" :aria-busy="busy">
    <header class="section-header mcp-header">
      <div>
        <h2 class="section-title">{{ MCP_TEXT.title }}</h2>
        <p class="section-subtitle">连接本机 AI 工具，使用剪贴板与 OCR。</p>
      </div>
      <div class="mcp-status-controls">
        <span class="service-status" :class="serviceState" role="status">{{
          loading
            ? MCP_TEXT.loading
            : !loaded
              ? '状态未知'
              : status.error
                ? '服务异常'
                : status.running
                  ? MCP_TEXT.running
                  : MCP_TEXT.stopped
        }}</span>
        <button
          type="button"
          class="status-refresh"
          :disabled="busy"
          :aria-label="MCP_TEXT.refresh"
          :title="MCP_TEXT.refresh"
          @click="refresh"
        >
          <IconRefresh />
        </button>
      </div>
    </header>
    <p v-if="error || status.error" class="mcp-error" role="alert">{{ error || status.error }}</p>

    <form class="mcp-service" @submit.prevent="save">
      <fieldset :disabled="busy || !loaded" class="settings-grid mcp-fields" aria-label="服务设置">
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-enabled" class="setting-title">{{ MCP_TEXT.enabled }}</label>
            <p class="setting-description">保存后生效，下次启动自动恢复</p>
          </div>
          <label class="toggle-switch"
            ><input id="mcp-enabled" v-model="form.enabled" type="checkbox" /><span
              class="toggle-slider"
          /></label>
        </div>
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-port" class="setting-title">{{ MCP_TEXT.port }}</label>
            <p class="setting-description">本机地址 · 127.0.0.1</p>
          </div>
          <NumberInput
            id="mcp-port"
            :model-value="form.port"
            :min="1024"
            :max="65535"
            :step="1"
            :clamp-on-commit="false"
            required
            class="number-input"
            @update:model-value="updatePort"
          />
        </div>
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-auth" class="setting-title">{{ MCP_TEXT.auth }}</label>
            <p class="setting-description">
              {{ form.authEnabled ? '客户端连接时需要提供密钥' : '本机其他程序也可访问服务' }}
            </p>
          </div>
          <label class="toggle-switch"
            ><input id="mcp-auth" v-model="form.authEnabled" type="checkbox" /><span
              class="toggle-slider"
          /></label>
        </div>
        <div v-if="form.authEnabled" class="setting-card key-card">
          <div class="setting-info">
            <label for="mcp-key" class="setting-title">{{ MCP_TEXT.key }}</label>
            <p class="setting-description">8–256 位字符，仅保存在本机</p>
          </div>
          <div class="key-controls">
            <div class="key-input-row">
              <input
                id="mcp-key"
                v-model="form.apiKey"
                class="mcp-key"
                :type="showKey ? 'text' : 'password'"
                autocomplete="off"
                spellcheck="false"
                minlength="8"
                maxlength="256"
                :required="form.enabled && form.authEnabled"
              /><button
                type="button"
                class="key-visibility"
                :aria-label="showKey ? MCP_TEXT.hide : MCP_TEXT.reveal"
                :aria-pressed="showKey"
                @click="showKey = !showKey"
              >
                {{ showKey ? '隐藏' : '显示' }}
              </button>
            </div>
            <div class="key-actions">
              <button type="button" class="test-button" @click="generateKey">
                {{ MCP_TEXT.generate }}</button
              ><button
                type="button"
                class="test-button"
                :disabled="!form.apiKey"
                @click="copy(form.apiKey)"
              >
                {{ MCP_TEXT.copyKey }}
              </button>
            </div>
          </div>
        </div>
      </fieldset>
      <div class="mcp-save-bar">
        <span class="save-hint">{{
          loading
            ? MCP_TEXT.loading
            : !loaded
              ? '配置未加载'
              : dirty
                ? '有未保存的更改'
                : '配置已保存'
        }}</span
        ><button
          type="submit"
          class="test-button save-button"
          :disabled="busy || !loaded || (!dirty && !status.error)"
        >
          {{ saving ? MCP_TEXT.saving : MCP_TEXT.save }}
        </button>
      </div>
    </form>
    <div v-if="loaded" class="mcp-endpoint">
      <span>{{ MCP_TEXT.address }}</span
      ><code>{{ status.endpoint }}</code
      ><button
        type="button"
        class="status-refresh"
        :aria-label="`复制${MCP_TEXT.address}`"
        :title="`复制${MCP_TEXT.address}`"
        @click="copy(status.endpoint)"
      >
        <IconCopy />
      </button>
    </div>
    <MCPClientGuide v-if="loaded" :settings="saved" :disabled="busy" :copy="copy" />
    <MCPReference />
    <p v-if="message" class="mcp-feedback" role="status">{{ message }}</p>
  </section>
</template>

<style scoped>
.mcp-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  flex-wrap: wrap;
}
.mcp-status-controls {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 3px;
}
.service-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
  color: var(--color-text-secondary);
  font-size: 11px;
  padding: 4px 8px;
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-full);
  background: var(--color-bg-primary);
}
.service-status::before {
  content: '';
  width: 6px;
  height: 6px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--color-text-tertiary);
}
.service-status.running::before {
  background: var(--color-success);
}
.service-status.error::before {
  background: var(--color-danger);
}
.status-refresh {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  font-size: 15px;
  flex-shrink: 0;
}
.status-refresh:hover:not(:disabled) {
  color: var(--color-text-primary);
  background: var(--color-control-hover);
}
.mcp-fields {
  min-width: 0;
  padding: 0;
  margin: 0;
}
.setting-title {
  display: block;
}
.key-card {
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 12px;
}
.key-controls {
  flex: 1 1 250px;
  min-width: 0;
}
.key-input-row {
  display: flex;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
}
.key-input-row:focus-within {
  border-color: var(--color-primary);
}
.mcp-key {
  flex: 1;
  width: 0;
  min-width: 0;
  padding: 6px 9px;
  border: 0;
  background: transparent;
  color: var(--color-text-primary);
  font-family: var(--font-family-mono, monospace);
  font-size: 12px;
}
.key-visibility {
  border: 0;
  background: transparent;
  color: var(--color-text-secondary);
  padding: 4px 9px;
  font-size: 11px;
  cursor: pointer;
}
.key-actions {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
.mcp-save-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 12px;
}
.save-hint {
  color: var(--color-text-tertiary);
  font-size: 11px;
}
.save-button {
  background: var(--color-primary);
  color: var(--color-text-inverse);
}
.save-button:hover:not(:disabled) {
  background: var(--color-primary-hover);
}
.mcp-settings button:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}
.mcp-endpoint {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 18px;
  padding-block: 10px;
  border-block: 1px solid var(--color-border-light);
  font-size: 11px;
  color: var(--color-text-secondary);
}
.mcp-endpoint > span {
  flex-shrink: 0;
}
.mcp-endpoint code {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
  -webkit-user-select: text;
  user-select: text;
}
.mcp-error {
  margin: 0 0 14px;
  padding: 10px 12px;
  border: 1px solid var(--color-danger);
  border-radius: var(--radius-sm);
  color: var(--color-danger);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.mcp-feedback {
  position: sticky;
  bottom: 0;
  width: fit-content;
  margin: 12px auto 0;
  padding: 6px 14px;
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-sm);
  background: var(--color-bg-elevated);
  box-shadow: var(--shadow-control);
  color: var(--color-text-secondary);
  font-size: 12px;
}
</style>
