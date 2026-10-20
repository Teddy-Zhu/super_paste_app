<script setup lang="ts">
import { ref } from 'vue'
import { STATISTICS_TEXT as text } from '@renderer/constants/statistics'
import CopyStatistics from './CopyStatistics.vue'
import AIOCRUsage from './AIOCRUsage.vue'

const activeTab = ref<'copy' | 'token'>('copy')
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">{{ text.title }}</h3>
      <p class="section-subtitle">{{ text.description }}</p>
    </div>
    <div class="statistics-tabs">
      <button
        v-for="tab in ['copy', 'token'] as const"
        :key="tab"
        type="button"
        :class="{ active: activeTab === tab }"
        :aria-pressed="activeTab === tab"
        @click="activeTab = tab"
      >
        {{ tab === 'copy' ? text.copyTab : text.tokenTab }}
      </button>
    </div>
    <div class="settings-grid">
      <CopyStatistics v-if="activeTab === 'copy'" />
      <AIOCRUsage v-else />
    </div>
  </div>
</template>

<style scoped>
.statistics-tabs {
  display: flex;
  gap: 8px;
  margin-bottom: 20px;
  border-bottom: 1px solid #e2e8f0;
}
.statistics-tabs button {
  padding: 10px 18px;
  border: 0;
  border-bottom: 2px solid transparent;
  background: transparent;
  color: #64748b;
  cursor: pointer;
  font-size: 14px;
}
.statistics-tabs button.active {
  border-bottom-color: var(--color-primary, #2563eb);
  color: var(--color-primary, #2563eb);
}
</style>
