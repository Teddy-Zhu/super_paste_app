<script setup lang="ts">
import IconChevron from '~icons/mdi/chevron-right'
import { MCP_TEXT, MCP_TOOLS } from '@shared/mcp'
import { MCP_PUSH_EXAMPLE } from '@renderer/constants/mcp'
</script>

<template>
  <details class="mcp-reference">
    <summary class="reference-heading">
      <IconChevron /><span>工具与使用说明</span
      ><small>{{ Object.keys(MCP_TOOLS).length }} 项工具</small>
    </summary>
    <div class="reference-content">
      <p class="reference-notice">{{ MCP_TEXT.localNotice }}</p>
      <p class="reference-notice">{{ MCP_TEXT.accessNotice }}</p>
      <div class="tool-list">
        <details v-for="(tool, name) in MCP_TOOLS" :key="name" class="tool-item">
          <summary>
            <IconChevron /><span>{{ tool.title }}</span
            ><code>{{ name }}</code>
          </summary>
          <p>{{ tool.description }}</p>
        </details>
      </div>
      <p class="reference-notice">{{ MCP_TEXT.ocrNotice }}</p>
      <details class="reference-section">
        <summary><IconChevron />{{ MCP_TEXT.pushTitle }}</summary>
        <div class="protocol-details">
          <p>{{ MCP_TEXT.pushDescription }}</p>
          <p>{{ MCP_TEXT.pushConnection }}</p>
          <pre><code>{{ MCP_PUSH_EXAMPLE }}</code></pre>
          <p>{{ MCP_TEXT.pushCancel }}</p>
          <p>{{ MCP_TEXT.pushCompatibility }}</p>
          <p>{{ MCP_TEXT.changesNotice }}</p>
        </div>
      </details>
      <details class="reference-section">
        <summary><IconChevron />{{ MCP_TEXT.promptTitle }}</summary>
        <p class="prompt-example">{{ MCP_TEXT.prompt }}</p>
      </details>
    </div>
  </details>
</template>

<style scoped>
.mcp-reference {
  margin-top: 24px;
  border-top: 1px solid var(--color-border-medium);
}
summary {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 11px 0;
  list-style: none;
  cursor: pointer;
  font-size: 12px;
}
summary::-webkit-details-marker {
  display: none;
}
summary > svg {
  flex-shrink: 0;
  color: var(--color-text-secondary);
  transition: transform var(--duration-fast);
}
details[open] > summary > svg {
  transform: rotate(90deg);
}
.reference-heading {
  padding-block: 16px;
  font-size: 13px;
  font-weight: 500;
}
.reference-heading small {
  margin-left: auto;
  font-size: 11px;
  color: var(--color-text-tertiary);
  font-weight: 400;
}
.reference-content {
  padding: 0 4px 4px;
}
.reference-notice {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--color-text-secondary);
}
.tool-list {
  margin-bottom: 12px;
  border: 1px solid var(--color-border-medium);
  border-radius: var(--radius-sm);
  background: var(--color-bg-primary);
}
.tool-item {
  padding: 0 12px;
}
.tool-item + .tool-item {
  border-top: 1px solid var(--color-border-light);
}
.tool-item summary {
  flex-wrap: wrap;
  gap: 5px 8px;
}
.tool-item summary code {
  margin-left: auto;
  color: var(--color-text-tertiary);
  font-size: 10px;
  overflow-wrap: anywhere;
}
.tool-item p,
.protocol-details p,
.prompt-example {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.7;
  color: var(--color-text-secondary);
  overflow-wrap: anywhere;
}
.tool-item p {
  padding-left: 20px;
}
.reference-section {
  border-top: 1px solid var(--color-border-light);
}
.protocol-details,
.prompt-example {
  padding-inline: 20px;
}
pre {
  margin: 12px 0;
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
</style>
