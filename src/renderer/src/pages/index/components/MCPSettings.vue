<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke, onSimple } from '@renderer/api/ipc'
import {
  createDefaultMCPSettings,
  getMCPEndpoint,
  MCP_TEXT,
  MCP_TOOLS,
  type MCPStatus
} from '@shared/mcp'
import {
  createMCPClientConfig,
  MCP_CLIENTS,
  MCP_PUSH_EXAMPLE,
  type MCPClientId
} from '@renderer/constants/mcp'

const form = ref(createDefaultMCPSettings())
const saved = ref(createDefaultMCPSettings())
const status = ref<MCPStatus>({ running: false, endpoint: getMCPEndpoint(saved.value.port) })
const loading = ref(true)
const saving = ref(false)
const showKey = ref(false)
const error = ref('')
const message = ref('')
const clientId = ref<MCPClientId>('cursor')
const client = computed(() => MCP_CLIENTS.find((item) => item.id === clientId.value)!)
const config = computed(() => createMCPClientConfig(clientId.value, saved.value, showKey.value))
const busy = computed(() => loading.value || saving.value)
let unsubscribe: (() => void) | undefined

async function refresh(): Promise<void> {
  error.value = ''
  try {
    const response = await invoke('mcp:get-state')
    if (!response.success || !response.data) throw new Error(response.error)
    saved.value = { ...response.data.settings }
    status.value = response.data.status
    if (loading.value) form.value = { ...response.data.settings }
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    loading.value = false
  }
}

async function save(): Promise<void> {
  saving.value = true
  error.value = ''
  message.value = ''
  try {
    const response = await invoke('mcp:save-settings', { ...form.value })
    if (!response.success || !response.data) throw new Error(response.error)
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
  error.value = ''
  try {
    const response = await invoke('mcp:generate-key')
    if (!response.success || !response.data) throw new Error(response.error)
    form.value.apiKey = response.data
    message.value = ''
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  }
}

async function copy(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text)
    message.value = MCP_TEXT.copied
  } catch {
    error.value = MCP_TEXT.copyFailed
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
    <header class="section-header">
      <h2 class="section-title">{{ MCP_TEXT.title }}</h2>
      <p class="section-subtitle">{{ MCP_TEXT.subtitle }}</p>
    </header>

    <p v-if="loading" role="status">{{ MCP_TEXT.loading }}</p>
    <form @submit.prevent="save">
      <fieldset :disabled="busy" class="settings-grid mcp-fields">
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-enabled" class="setting-title">{{ MCP_TEXT.enabled }}</label>
            <p class="setting-description">{{ MCP_TEXT.enabledDescription }}</p>
          </div>
          <label class="toggle-switch">
            <input id="mcp-enabled" v-model="form.enabled" type="checkbox" />
            <span class="toggle-slider" />
          </label>
        </div>
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-port" class="setting-title">{{ MCP_TEXT.port }}</label>
            <p class="setting-description">{{ MCP_TEXT.portDescription }}</p>
          </div>
          <input
            id="mcp-port"
            v-model.number="form.port"
            class="number-input"
            type="number"
            min="1024"
            max="65535"
            step="1"
            required
          />
        </div>
        <div class="setting-card">
          <div class="setting-info">
            <label for="mcp-auth" class="setting-title">{{ MCP_TEXT.auth }}</label>
            <p class="setting-description">{{ MCP_TEXT.authDescription }}</p>
          </div>
          <label class="toggle-switch">
            <input id="mcp-auth" v-model="form.authEnabled" type="checkbox" />
            <span class="toggle-slider" />
          </label>
        </div>
        <div class="setting-card key-card">
          <div class="setting-info">
            <label for="mcp-key" class="setting-title">{{ MCP_TEXT.key }}</label>
            <p class="setting-description">{{ MCP_TEXT.keyDescription }}</p>
          </div>
          <div class="key-controls">
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
            />
            <div class="mcp-actions">
              <button type="button" class="test-button" @click="generateKey">
                {{ MCP_TEXT.generate }}
              </button>
              <button type="button" class="test-button" @click="showKey = !showKey">
                {{ showKey ? MCP_TEXT.hide : MCP_TEXT.reveal }}
              </button>
              <button
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
      <div class="mcp-toolbar">
        <span class="service-status" :class="{ running: status.running }">{{
          status.running ? MCP_TEXT.running : MCP_TEXT.stopped
        }}</span>
        <button type="button" class="test-button" :disabled="busy" @click="refresh">
          {{ MCP_TEXT.refresh }}
        </button>
        <button type="submit" class="test-button save-button" :disabled="busy">
          {{ saving ? MCP_TEXT.saving : MCP_TEXT.save }}
        </button>
      </div>
    </form>

    <p v-if="error || status.error" class="mcp-error" role="alert">{{ error || status.error }}</p>
    <p v-if="message" class="mcp-message" role="status">{{ message }}</p>
    <div class="mcp-endpoint">
      <span>{{ MCP_TEXT.address }}</span>
      <code>{{ status.endpoint }}</code>
      <button type="button" class="test-button" @click="copy(status.endpoint)">
        {{ MCP_TEXT.copy }}
      </button>
    </div>
    <p class="setting-description mcp-note">{{ MCP_TEXT.localNotice }}</p>
    <p class="setting-description mcp-note">{{ MCP_TEXT.accessNotice }}</p>

    <section class="mcp-group">
      <h3 class="setting-title">{{ MCP_TEXT.capabilities }}</h3>
      <dl class="tool-list settings-grid">
        <div v-for="(tool, name) in MCP_TOOLS" :key="name" class="tool-row">
          <dt>
            <strong>{{ tool.title }}</strong
            ><code>{{ name }}</code>
          </dt>
          <dd>{{ tool.description }}</dd>
        </div>
      </dl>
      <p class="setting-description mcp-note">{{ MCP_TEXT.ocrNotice }}</p>
      <p class="setting-description mcp-note">{{ MCP_TEXT.changesNotice }}</p>
    </section>

    <section class="mcp-group">
      <h3 class="setting-title">{{ MCP_TEXT.pushTitle }}</h3>
      <p class="setting-description mcp-note">{{ MCP_TEXT.pushDescription }}</p>
      <p class="setting-description mcp-note">{{ MCP_TEXT.pushConnection }}</p>
      <pre><code>{{ MCP_PUSH_EXAMPLE }}</code></pre>
      <p class="setting-description mcp-note">{{ MCP_TEXT.pushCancel }}</p>
      <p class="setting-description mcp-note">{{ MCP_TEXT.pushCompatibility }}</p>
    </section>
    <section class="mcp-group">
      <h3 class="setting-title">{{ MCP_TEXT.clients }}</h3>
      <p class="setting-description">{{ MCP_TEXT.clientDescription }}</p>
      <div class="client-tabs" role="tablist" :aria-label="MCP_TEXT.clients">
        <button
          v-for="item in MCP_CLIENTS"
          :id="`mcp-tab-${item.id}`"
          :key="item.id"
          type="button"
          class="test-button"
          role="tab"
          :aria-selected="clientId === item.id"
          aria-controls="mcp-client-panel"
          @click="clientId = item.id"
        >
          {{ item.name }}
        </button>
      </div>
      <div
        id="mcp-client-panel"
        class="client-panel"
        role="tabpanel"
        :aria-labelledby="`mcp-tab-${clientId}`"
      >
        <p class="setting-description">{{ client.help }}</p>
        <pre><code>{{ config }}</code></pre>
        <div class="mcp-toolbar">
          <a :href="client.docs" target="_blank" rel="noreferrer">{{ MCP_TEXT.docs }}</a>
          <button
            type="button"
            class="test-button"
            :disabled="busy"
            @click="copy(createMCPClientConfig(clientId, saved, true))"
          >
            {{ MCP_TEXT.copyConfig }}
          </button>
        </div>
        <p v-if="saved.authEnabled && clientId !== 'codex'" class="setting-description mcp-note">
          {{ MCP_TEXT.configSecretNotice }}
        </p>
      </div>
    </section>
    <section class="mcp-group">
      <h3 class="setting-title">{{ MCP_TEXT.promptTitle }}</h3>
      <p class="setting-description">{{ MCP_TEXT.prompt }}</p>
    </section>
  </section>
</template>

<style scoped>
.mcp-fields {
  padding: 0;
  margin: 0;
  min-width: 0;
}
.setting-title {
  display: block;
}
.key-card {
  align-items: flex-start;
  flex-wrap: wrap;
}
.key-controls {
  flex: 1 1 290px;
  min-width: 0;
}
.mcp-key {
  width: 100%;
  min-width: 0;
  padding: 6px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  font-family: monospace;
}
.mcp-actions,
.mcp-toolbar,
.client-tabs,
.mcp-endpoint {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
.mcp-actions {
  margin-top: 8px;
}
.mcp-toolbar {
  margin-top: 14px;
}
.save-button {
  margin-left: auto;
  background: var(--color-primary);
  color: var(--color-text-inverse);
}
.save-button:hover {
  background: var(--color-primary-hover);
}
.mcp-settings button:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}
.service-status {
  font-size: 12px;
  color: var(--color-text-secondary);
}
.service-status::before {
  content: '';
  display: inline-block;
  margin-right: 6px;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--color-text-tertiary);
}
.service-status.running::before {
  background: var(--color-success, #3c995b);
}
.mcp-endpoint {
  margin-top: 18px;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: var(--color-bg-secondary);
  font-size: 12px;
}
.mcp-endpoint code {
  overflow-wrap: anywhere;
}
.mcp-note {
  margin-top: 10px;
}
.mcp-group {
  margin-top: 28px;
}
.tool-list {
  margin: 12px 0 0;
}
.tool-row {
  padding: 12px 16px;
}
.tool-row + .tool-row {
  border-top: 1px solid var(--color-border-light);
}
.tool-row dt {
  display: flex;
  gap: 12px;
  align-items: baseline;
  flex-wrap: wrap;
}
.tool-row strong {
  font-size: 12px;
  font-weight: 500;
}
.tool-row code {
  font-size: 11px;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}
.tool-row dd {
  margin: 6px 0 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--color-text-secondary);
}
.client-tabs {
  margin: 12px 0;
}
.client-tabs [aria-selected='true'] {
  background: var(--color-selection);
  border-color: var(--color-primary);
}
.client-panel {
  padding: 16px;
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-md);
  background: var(--color-bg-primary);
}
pre {
  padding: 12px;
  margin: 12px 0 0;
  background: var(--color-bg-secondary);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  font-size: 12px;
  line-height: 1.6;
  user-select: text;
}
.client-panel .mcp-toolbar {
  justify-content: space-between;
  font-size: 12px;
}
.client-panel a {
  color: var(--color-primary);
}
.mcp-error {
  color: var(--color-danger);
  font-size: 12px;
  overflow-wrap: anywhere;
}
.mcp-message {
  color: var(--color-text-secondary);
  font-size: 12px;
}
</style>
