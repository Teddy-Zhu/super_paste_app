<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke, onSimple } from '@renderer/api/ipc'
import { OCR_USAGE_TEXT as text } from '@renderer/constants/ocr'
import { STATISTICS_TEXT } from '@renderer/constants/statistics'
import { createTokenTrend } from '@renderer/utils/statistics'
import StatisticsLineChart from '@renderer/components/StatisticsLineChart.vue'
import type { AIOCRUsageStats } from '@shared/ai-ocr'

const stats = ref<AIOCRUsageStats>()
const period = ref('all')
const loading = ref(false)
const clearing = ref(false)
const confirmingClear = ref(false)
const error = ref('')
let version = 0
let disposed = false
let unsubscribe: (() => void) | undefined
let refreshTimer: ReturnType<typeof setTimeout> | undefined
const number = (value: number | null | undefined): string =>
  value == null ? '—' : value.toLocaleString()
const time = (value: number | null | undefined): string =>
  value == null ? '—' : new Date(value).toLocaleString()
const seconds = (value: number): string => `${(value / 1000).toFixed(2)} s`
const rangeStart = (): number | undefined => {
  if (period.value === 'all') return undefined
  const date = new Date()
  date.setHours(0, 0, 0, 0)
  date.setDate(date.getDate() - (period.value === 'week' ? 6 : 29))
  return date.getTime()
}
const trend = computed(() => createTokenTrend(stats.value?.days ?? [], rangeStart()))
const metrics = computed(() => {
  const totals = stats.value?.totals
  if (!totals) return []
  return [
    [text.total, number(totals.totalTokens)],
    [text.input, number(totals.inputTokens)],
    [text.output, number(totals.outputTokens)],
    [text.requests, number(totals.requests)],
    [text.failures, number(totals.failures)],
    [text.unknown, number(totals.unknownUsage)],
    [text.cacheRead, number(totals.cachedInputTokens)],
    [text.cacheWrite, number(totals.cacheWriteTokens)],
    [text.duration, seconds(totals.durationMs)]
  ]
})

const load = async (): Promise<void> => {
  const current = ++version
  loading.value = true
  error.value = ''
  try {
    const result = await invoke('ocr:get-usage', { from: rangeStart() })
    if (disposed || current !== version) return
    if (!result.success || !result.data) throw new Error(result.error || text.loadFailed)
    stats.value = result.data
  } catch (cause) {
    if (!disposed && current === version)
      error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (current === version) loading.value = false
  }
}

const clear = async (): Promise<void> => {
  clearing.value = true
  error.value = ''
  ++version
  try {
    const result = await invoke('ocr:clear-usage')
    if (!result.success) throw new Error(result.error || text.clearFailed)
    confirmingClear.value = false
    await load()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    clearing.value = false
  }
}

watch(period, load)
onMounted(() => {
  unsubscribe = onSimple('ocr:usage-updated', () => {
    if (refreshTimer) clearTimeout(refreshTimer)
    refreshTimer = setTimeout(() => {
      void load()
    }, 150)
  })
  void load()
})
onBeforeUnmount(() => {
  disposed = true
  unsubscribe?.()
  if (refreshTimer) clearTimeout(refreshTimer)
})
</script>

<template>
  <div class="setting-card usage-card">
    <div class="usage-content">
      <h4 class="setting-title">{{ text.title }}</h4>
      <p class="setting-description">{{ text.description }}</p>
      <div class="usage-actions">
        <select v-model="period" :disabled="clearing" :aria-label="text.time">
          <option value="all">{{ text.all }}</option>
          <option value="week">{{ text.week }}</option>
          <option value="month">{{ text.month }}</option>
        </select>
        <button class="shortcut-button secondary" :disabled="loading || clearing" @click="load">
          {{ text.refresh }}
        </button>
        <button
          class="shortcut-button secondary"
          :disabled="clearing"
          @click="confirmingClear = true"
        >
          {{ text.clear }}
        </button>
      </div>
      <div v-if="confirmingClear" class="clear-confirm" role="alert">
        <p>{{ text.clearConfirm }}</p>
        <div class="usage-actions">
          <button class="shortcut-button secondary" :disabled="clearing" @click="clear">
            {{ text.confirmClear }}
          </button>
          <button
            class="shortcut-button secondary"
            :disabled="clearing"
            @click="confirmingClear = false"
          >
            {{ text.cancel }}
          </button>
        </div>
      </div>
      <p v-if="error" class="usage-error" role="alert">{{ error }}</p>
      <template v-if="stats">
        <dl class="usage-metrics">
          <div v-for="[label, value] in metrics" :key="label">
            <dt>{{ label }}</dt>
            <dd>{{ value }}</dd>
          </div>
        </dl>
        <p class="usage-time">
          {{ text.firstUsed }}: {{ time(stats.totals.firstUsedAt) }} · {{ text.lastUsed }}:
          {{ time(stats.totals.lastUsedAt) }}
        </p>
        <p v-if="stats.clearedAt" class="usage-time">
          {{ text.clearedAt }}: {{ time(stats.clearedAt) }}
        </p>
        <p v-if="!stats.totals.requests" class="usage-time">{{ text.empty }}</p>
        <h5>{{ STATISTICS_TEXT.tokenTrend }}</h5>
        <StatisticsLineChart
          :points="trend.points"
          :label="text.total"
          :interval="trend.interval"
        />
        <template v-if="stats.totals.requests">
          <h5>{{ text.byModel }}</h5>
          <div class="usage-table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{{ text.model }}</th>
                  <th>{{ text.requests }}</th>
                  <th>{{ text.input }}</th>
                  <th>{{ text.output }}</th>
                  <th>{{ text.total }}</th>
                  <th>{{ text.lastUsed }}</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="model in stats.models"
                  :key="`${model.provider}:${model.baseUrl}:${model.model}`"
                >
                  <td>
                    {{ model.model }}<small>{{ model.provider }} · {{ model.baseUrl }}</small>
                  </td>
                  <td>
                    {{ number(model.requests)
                    }}<small
                      >{{ text.failures }}: {{ model.failures }} · {{ text.unknown }}:
                      {{ model.unknownUsage }}</small
                    >
                  </td>
                  <td>{{ number(model.inputTokens) }}</td>
                  <td>{{ number(model.outputTokens) }}</td>
                  <td>{{ number(model.totalTokens) }}</td>
                  <td>{{ time(model.lastUsedAt) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <details>
            <summary>{{ text.byDay }}</summary>
            <div class="usage-table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>{{ text.day }}</th>
                    <th>{{ text.requests }}</th>
                    <th>{{ text.input }}</th>
                    <th>{{ text.output }}</th>
                    <th>{{ text.total }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="day in stats.days" :key="day.day">
                    <td>{{ day.day }}</td>
                    <td>{{ number(day.requests) }}</td>
                    <td>{{ number(day.inputTokens) }}</td>
                    <td>{{ number(day.outputTokens) }}</td>
                    <td>{{ number(day.totalTokens) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </details>
          <details>
            <summary>{{ text.recent }}</summary>
            <div class="usage-table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>{{ text.time }}</th>
                    <th>{{ text.model }}</th>
                    <th>{{ text.purpose }}</th>
                    <th>{{ text.status }}</th>
                    <th>{{ text.input }}</th>
                    <th>{{ text.output }}</th>
                    <th>{{ text.total }}</th>
                    <th>{{ text.duration }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="entry in stats.recent" :key="entry.id">
                    <td>{{ time(entry.startedAt) }}</td>
                    <td>
                      {{ entry.model }}<small>{{ entry.provider }} · {{ entry.baseUrl }}</small>
                    </td>
                    <td>{{ entry.purpose === 'ocr' ? text.ocr : text.test }}</td>
                    <td>{{ entry.success ? text.success : text.failed }}</td>
                    <td>{{ number(entry.inputTokens) }}</td>
                    <td>{{ number(entry.outputTokens) }}</td>
                    <td>{{ number(entry.totalTokens) }}</td>
                    <td>{{ seconds(entry.durationMs) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </details>
        </template>
      </template>
    </div>
  </div>
</template>

<style scoped>
.usage-card {
  align-items: flex-start;
}
.usage-content {
  width: 100%;
  min-width: 0;
}
.usage-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 12px;
}
select {
  padding: 5px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
}
.usage-metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(115px, 1fr));
  gap: 14px;
  margin: 18px 0;
}
dt,
small,
.usage-time {
  color: var(--color-text-secondary);
  font-size: 12px;
}
dd {
  margin: 6px 0 0;
  font-size: 18px;
  font-variant-numeric: tabular-nums;
}
.usage-time {
  margin-top: 8px;
}
.usage-table-wrap {
  width: 100%;
  overflow: auto;
  margin-top: 10px;
}
table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
  text-align: left;
}
th,
td {
  padding: 10px 8px;
  border-bottom: 1px solid var(--color-control-border);
  vertical-align: top;
}
td:first-child {
  min-width: 130px;
}
small {
  display: block;
  margin-top: 4px;
  overflow-wrap: anywhere;
}
h5,
details {
  margin-top: 18px;
}
summary {
  cursor: pointer;
  font-size: 13px;
}
.usage-error {
  color: var(--color-danger);
  margin-top: 12px;
  font-size: 12px;
}
.clear-confirm {
  font-size: 12px;
  margin-top: 12px;
}
button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
