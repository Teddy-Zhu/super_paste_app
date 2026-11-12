<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import IconCopy from '~icons/mdi/content-copy'
import IconChevron from '~icons/mdi/chevron-right'
import { send } from '@renderer/api/ipc'
import { createMCPClientConfig, MCP_CLIENTS, type MCPClientId } from '@renderer/constants/mcp'
import { MCP_TEXT, type MCPSettings } from '@shared/mcp'

const props = defineProps<{
  settings: MCPSettings
  disabled: boolean
  copy: (text: string) => Promise<boolean>
}>()
const clientId = ref<MCPClientId>('cursor')
const revealKey = ref(false)
const copied = ref(false)
const client = computed(() => MCP_CLIENTS.find((item) => item.id === clientId.value)!)
const config = computed(() =>
  createMCPClientConfig(clientId.value, props.settings, revealKey.value)
)
watch([clientId, () => props.settings], () => {
  copied.value = false
  revealKey.value = false
})
const copyConfig = async (): Promise<void> => {
  copied.value = await props.copy(createMCPClientConfig(clientId.value, props.settings, true))
}
const navigateClient = (event: KeyboardEvent): void => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  const current = MCP_CLIENTS.findIndex((item) => item.id === clientId.value)
  const index =
    event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? MCP_CLIENTS.length - 1
        : (current + (event.key === 'ArrowRight' ? 1 : -1) + MCP_CLIENTS.length) %
          MCP_CLIENTS.length
  clientId.value = MCP_CLIENTS[index].id
  const target = event.currentTarget as HTMLElement
  target.querySelector<HTMLButtonElement>(`#mcp-tab-${clientId.value}`)?.focus()
}
</script>

<template>
  <section class="mcp-connect" aria-labelledby="mcp-connect-title">
    <div class="mcp-section-heading">
      <h3 id="mcp-connect-title">连接客户端</h3>
      <span>使用已保存的服务配置</span>
    </div>
    <div class="client-card">
      <div
        class="client-tabs"
        role="tablist"
        :aria-label="MCP_TEXT.clients"
        @keydown="navigateClient"
      >
        <button
          v-for="item in MCP_CLIENTS"
          :id="`mcp-tab-${item.id}`"
          :key="item.id"
          type="button"
          role="tab"
          :aria-selected="clientId === item.id"
          :tabindex="clientId === item.id ? 0 : -1"
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
        tabindex="0"
      >
        <p class="client-help">{{ client.help }}</p>
        <div class="client-actions">
          <a :href="client.docs" @click.prevent="send('open-external-url', client.docs)">{{
            MCP_TEXT.docs
          }}</a>
          <button type="button" class="test-button" :disabled="disabled" @click="copyConfig">
            <IconCopy />{{ copied ? MCP_TEXT.copied : MCP_TEXT.copyConfig }}
          </button>
        </div>
        <p v-if="settings.authEnabled && clientId !== 'codex'" class="config-key-hint">
          配置包含 API Key
        </p>
        <details class="client-config">
          <summary><IconChevron />查看配置</summary>
          <div v-if="settings.authEnabled && clientId !== 'codex'" class="config-security">
            <span>{{ MCP_TEXT.configSecretNotice }}</span
            ><button type="button" class="key-toggle" @click="revealKey = !revealKey">
              {{ revealKey ? MCP_TEXT.hide : MCP_TEXT.reveal }}
            </button>
          </div>
          <pre><code>{{ config }}</code></pre>
        </details>
      </div>
    </div>
  </section>
</template>

<style scoped>
.mcp-section-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  margin: 24px 0 10px;
}
.mcp-section-heading h3 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}
.mcp-section-heading > span {
  font-size: 11px;
  color: var(--color-text-tertiary);
}
.client-card {
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-md);
  background: var(--color-bg-primary);
  overflow: hidden;
}
.client-tabs {
  display: flex;
  gap: 4px;
  padding: 0 12px;
  border-bottom: 1px solid var(--color-border-light);
  overflow-x: auto;
}
.client-tabs button {
  position: relative;
  flex-shrink: 0;
  padding: 12px 10px;
  border: 0;
  background: transparent;
  font-size: 12px;
  color: var(--color-text-secondary);
  cursor: pointer;
}
.client-tabs button[aria-selected='true'] {
  color: var(--color-primary);
  font-weight: 600;
}
.client-tabs button[aria-selected='true']::after {
  content: '';
  position: absolute;
  height: 2px;
  bottom: 0;
  left: 10px;
  right: 10px;
  background: var(--color-primary);
}
.client-panel {
  padding: 16px;
}
.client-help {
  margin: 0;
  font-size: 12px;
  line-height: 1.7;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}
.client-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 14px;
}
.client-actions a,
.key-toggle {
  font-size: 12px;
  color: var(--color-primary);
}
.client-actions button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.config-key-hint {
  margin: 8px 0 0;
  font-size: 11px;
  text-align: right;
  color: var(--color-text-tertiary);
}
.client-config {
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px solid var(--color-border-light);
}
summary {
  display: flex;
  align-items: center;
  gap: 5px;
  list-style: none;
  font-size: 12px;
  color: var(--color-text-secondary);
  cursor: pointer;
}
summary::-webkit-details-marker {
  display: none;
}
summary svg {
  transition: transform var(--duration-fast);
}
details[open] > summary svg {
  transform: rotate(90deg);
}
.config-security {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 8px;
  margin-top: 12px;
  font-size: 11px;
  color: var(--color-text-tertiary);
}
.key-toggle {
  border: 0;
  background: transparent;
  padding: 0;
  cursor: pointer;
}
pre {
  margin: 12px 0 0;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: var(--color-bg-secondary);
  font-size: 11px;
  line-height: 1.65;
  overflow-x: auto;
}
pre,
code {
  -webkit-user-select: text;
  user-select: text;
}
button:disabled {
  opacity: var(--opacity-disabled);
  cursor: default;
}
</style>
