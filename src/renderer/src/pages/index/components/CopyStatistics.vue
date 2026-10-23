<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke, onSimple } from '@renderer/api/ipc'
import { STATISTICS_TEXT as text } from '@renderer/constants/statistics'
import { PREVIEW_TYPES } from '@renderer/constants/clipboard-preview'
import { formatStatisticsBytes } from '@renderer/utils/statistics'
import StatisticsLineChart from '@renderer/components/StatisticsLineChart.vue'
import {
  COPY_STATISTICS_TYPES,
  type CopyStatistics,
  type CopyStatisticsQuery,
  type CopyStatisticsType,
  type StatisticsInterval
} from '@shared/clipboard-statistics'

const stats = ref<CopyStatistics>()
const period = ref('week')
const type = ref<CopyStatisticsType | ''>('')
const source = ref<string>()
const interval = ref<StatisticsInterval>('day')
const metric = ref<'copies' | 'bytes' | 'files'>('copies')
const startDate = ref('')
const endDate = ref('')
const loading = ref(false)
const clearing = ref(false)
const confirmingClear = ref(false)
const error = ref('')
let version = 0
let disposed = false
let unsubscribe: (() => void) | undefined
let refreshTimer: ReturnType<typeof setTimeout> | undefined
const number = (value: number): string => value.toLocaleString()
const time = (value: number | null): string =>
  value === null ? '—' : new Date(value).toLocaleString()
const sourceLabel = (value: string): string => value || text.unknownSource
const typeLabel = (value: string): string => PREVIEW_TYPES[value]?.label ?? value
const availableSources = computed(() => [
  ...new Set([
    ...(stats.value?.availableSources ?? []),
    ...(source.value === undefined ? [] : [source.value])
  ])
])
const trend = computed(
  () =>
    stats.value?.series.map((point) => ({
      timestamp: point.timestamp,
      value: point[metric.value]
    })) ?? []
)
const metrics = computed(() =>
  stats.value
    ? [
        [text.copies, number(stats.value.copies)],
        [text.bytes, formatStatisticsBytes(stats.value.bytes)],
        [text.files, number(stats.value.files)],
        [text.activeDays, number(stats.value.activeDays)]
      ]
    : []
)
const breakdowns = computed(() =>
  stats.value
    ? [
        { title: text.byType, label: text.type, rows: stats.value.types, format: typeLabel },
        { title: text.bySource, label: text.source, rows: stats.value.sources, format: sourceLabel }
      ]
    : []
)

const getQuery = (): CopyStatisticsQuery => {
  const query: CopyStatisticsQuery = { interval: interval.value }
  if (type.value) query.type = type.value
  if (source.value !== undefined) query.source = source.value
  if (period.value === 'all') return query
  const start = new Date()
  start.setHours(0, 0, 0, 0)
  const end = new Date(start)
  end.setDate(end.getDate() + 1)
  if (period.value === 'custom') {
    const from = new Date(`${startDate.value}T00:00:00`)
    const to = new Date(`${endDate.value}T00:00:00`)
    if (
      !startDate.value ||
      !endDate.value ||
      !Number.isFinite(from.getTime()) ||
      !Number.isFinite(to.getTime()) ||
      from > to
    )
      throw new Error(text.invalidDates)
    to.setDate(to.getDate() + 1)
    query.from = from.getTime()
    query.to = to.getTime()
  } else {
    start.setDate(
      start.getDate() - (period.value === 'week' ? 6 : period.value === 'month' ? 29 : 0)
    )
    query.from = start.getTime()
    query.to = end.getTime()
  }
  return query
}
const load = async (): Promise<void> => {
  const current = ++version
  loading.value = true
  error.value = ''
  try {
    const result = await invoke('statistics:get-copy', getQuery())
    if (disposed || current !== version) return
    if (!result.success || !result.data) throw new Error(result.error || text.loadFailed)
    stats.value = result.data
  } catch (cause) {
    if (!disposed && current === version) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      stats.value = undefined
    }
  } finally {
    if (current === version) loading.value = false
  }
}
const clear = async (): Promise<void> => {
  clearing.value = true
  ++version
  error.value = ''
  try {
    const result = await invoke('statistics:clear-copy')
    if (!result.success) throw new Error(result.error || text.clearFailed)
    confirmingClear.value = false
    await load()
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    clearing.value = false
  }
}
watch([period, type, source, interval, startDate, endDate], () => {
  confirmingClear.value = false
  void load()
})
onMounted(() => {
  unsubscribe = onSimple('statistics:copy-updated', () => {
    if (refreshTimer) return
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined
      void load()
    }, 250)
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
  <div class="setting-card copy-statistics" :aria-busy="loading">
    <h4 class="setting-title">{{ text.copyTab }}</h4>
    <p class="setting-description">{{ text.copyDescription }}</p>
    <div class="statistics-filters">
      <label
        >{{ text.time }}
        <select v-model="period" :disabled="clearing">
          <option value="today">{{ text.today }}</option>
          <option value="week">{{ text.week }}</option>
          <option value="month">{{ text.month }}</option>
          <option value="all">{{ text.all }}</option>
          <option value="custom">{{ text.custom }}</option>
        </select>
      </label>
      <template v-if="period === 'custom'">
        <label
          >{{ text.start }}<input v-model="startDate" type="date" :disabled="clearing"
        /></label>
        <label>{{ text.end }}<input v-model="endDate" type="date" :disabled="clearing" /></label>
      </template>
      <label
        >{{ text.type }}
        <select v-model="type" :disabled="clearing">
          <option value="">{{ text.allTypes }}</option>
          <option v-for="item in COPY_STATISTICS_TYPES" :key="item" :value="item">
            {{ typeLabel(item) }}
          </option>
        </select>
      </label>
      <label
        >{{ text.source }}
        <select v-model="source" :disabled="clearing">
          <option :value="undefined">{{ text.allSources }}</option>
          <option v-for="item in availableSources" :key="item" :value="item">
            {{ sourceLabel(item) }}
          </option>
        </select>
      </label>
      <label
        >{{ text.interval }}
        <select v-model="interval" :disabled="clearing">
          <option value="hour">{{ text.hour }}</option>
          <option value="day">{{ text.day }}</option>
          <option value="month">{{ text.byMonth }}</option>
        </select>
      </label>
      <button class="shortcut-button secondary" :disabled="loading || clearing" @click="load">
        {{ loading ? text.loading : text.refresh }}
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
      <div class="statistics-actions">
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
    <p v-if="error" class="statistics-error" role="alert">{{ error }}</p>
    <template v-if="stats">
      <dl class="statistics-metrics">
        <div v-for="[label, value] in metrics" :key="label">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </div>
      </dl>
      <p class="statistics-note">{{ text.bytesHint }}</p>
      <p class="statistics-note">
        {{ text.first }}: {{ time(stats.firstCopiedAt) }} · {{ text.last }}:
        {{ time(stats.lastCopiedAt) }}
      </p>
      <p v-if="stats.clearedAt" class="statistics-note">
        {{ text.clearedAt }}: {{ time(stats.clearedAt) }}
      </p>
      <div class="trend-heading">
        <h5>{{ text.trend }}</h5>
        <select v-model="metric" :aria-label="text.metric">
          <option value="copies">{{ text.copies }}</option>
          <option value="bytes">{{ text.bytes }}</option>
          <option value="files">{{ text.files }}</option>
        </select>
      </div>
      <StatisticsLineChart
        :points="trend"
        :label="text[metric]"
        :interval="stats.interval"
        :format-value="metric === 'bytes' ? formatStatisticsBytes : number"
      />
      <p v-if="!stats.copies" class="statistics-note">{{ text.empty }}</p>
      <template v-else>
        <section v-for="group in breakdowns" :key="group.title">
          <h5>{{ group.title }}</h5>
          <div class="statistics-table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{{ group.label }}</th>
                  <th>{{ text.copies }}</th>
                  <th>{{ text.share }}</th>
                  <th>{{ text.bytes }}</th>
                  <th>{{ text.files }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="row in group.rows" :key="row.key">
                  <td>{{ group.format(row.key) }}</td>
                  <td>{{ number(row.copies) }}</td>
                  <td>{{ ((row.copies / stats.copies) * 100).toFixed(1) }}%</td>
                  <td>{{ formatStatisticsBytes(row.bytes) }}</td>
                  <td>{{ number(row.files) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </template>
    </template>
    <p class="statistics-note collection-note">{{ text.copyNote }}</p>
  </div>
</template>

<style scoped>
.copy-statistics {
  display: block;
}
.statistics-filters,
.statistics-actions,
.trend-heading {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  align-items: end;
  margin-top: 16px;
}
label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--color-text-secondary);
  font-size: 12px;
}
select,
input {
  padding: 6px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-primary);
  color: var(--color-text-primary);
  max-width: 220px;
}
.statistics-metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: 16px;
  margin: 24px 0 16px;
}
dt,
.statistics-note {
  color: var(--color-text-secondary);
  font-size: 12px;
  line-height: 1.7;
}
dd {
  margin: 6px 0 0;
  font-size: 22px;
  font-variant-numeric: tabular-nums;
}
.trend-heading {
  align-items: center;
  margin-top: 24px;
}
.trend-heading h5 {
  margin-right: auto;
}
h5 {
  margin: 18px 0 10px;
  font-size: 14px;
}
.statistics-table-wrap {
  overflow-x: auto;
}
table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
th,
td {
  text-align: left;
  border-bottom: 1px solid var(--color-border-light);
  padding: 10px 8px;
  font-variant-numeric: tabular-nums;
}
td:first-child {
  overflow-wrap: anywhere;
  max-width: 240px;
}
.statistics-error {
  color: var(--color-danger);
  font-size: 13px;
}
.clear-confirm {
  margin-top: 16px;
  padding: 12px;
  background: var(--color-warning-bg);
  border: 1px solid var(--color-warning-border);
  border-radius: 8px;
  font-size: 13px;
}
.collection-note {
  margin: 20px 0 0;
}
</style>
