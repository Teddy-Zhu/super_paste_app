<script setup lang="ts">
import { computed } from 'vue'
import { STATISTICS_TEXT as text } from '@renderer/constants/statistics'
import type { StatisticsChartPoint } from '@renderer/utils/statistics'
import type { StatisticsInterval } from '@shared/clipboard-statistics'

const props = withDefaults(
  defineProps<{
    points: StatisticsChartPoint[]
    label: string
    interval?: StatisticsInterval
    formatValue?: (value: number) => string
  }>(),
  { interval: 'day', formatValue: (value: number) => value.toLocaleString() }
)
const max = computed(() =>
  Math.max(4, Math.ceil(Math.max(0, ...props.points.map((point) => point.value)) / 4) * 4)
)
const x = (index: number): number =>
  props.points.length <= 1 ? 360 : 64 + (index / (props.points.length - 1)) * 582
const y = (value: number): number => 186 - (value / max.value) * 164
const line = computed(() =>
  props.points.map((point, index) => `${x(index)},${y(point.value)}`).join(' ')
)
const ticks = computed(() => [0, 0.25, 0.5, 0.75, 1].map((part) => max.value * part))
const labels = computed(() =>
  [...new Set([0, Math.floor((props.points.length - 1) / 2), props.points.length - 1])].filter(
    (index) => index >= 0 && props.points[index]
  )
)
const dateLabel = (timestamp: number): string =>
  new Date(timestamp).toLocaleString(
    undefined,
    props.interval === 'hour'
      ? { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }
      : props.interval === 'month'
        ? { year: 'numeric', month: 'numeric' }
        : { month: 'numeric', day: 'numeric' }
  )
</script>

<template>
  <div class="statistics-chart">
    <p v-if="!points.length" class="chart-empty">{{ text.chartEmpty }}</p>
    <svg v-else viewBox="0 0 680 228" role="img" :aria-label="label">
      <title>{{ label }}</title>
      <g v-for="tick in ticks" :key="tick" class="chart-grid">
        <line x1="64" x2="646" :y1="y(tick)" :y2="y(tick)" />
        <text x="56" :y="y(tick) + 4" text-anchor="end">{{ formatValue(tick) }}</text>
      </g>
      <polyline :points="line" class="chart-line" />
      <circle
        v-for="(point, index) in points"
        :key="point.timestamp"
        :cx="x(index)"
        :cy="y(point.value)"
        r="3"
        class="chart-point"
      >
        <title>
          {{ new Date(point.timestamp).toLocaleString() }} · {{ label }}:
          {{ formatValue(point.value) }}
        </title>
      </circle>
      <text
        v-for="index in labels"
        :key="index"
        :x="x(index)"
        y="214"
        :text-anchor="index === 0 ? 'start' : index === points.length - 1 ? 'end' : 'middle'"
      >
        {{ dateLabel(points[index].timestamp) }}
      </text>
    </svg>
  </div>
</template>

<style scoped>
.statistics-chart {
  width: 100%;
  margin: 12px 0;
}
svg {
  display: block;
  width: 100%;
  min-height: 160px;
  max-height: 320px;
  overflow: visible;
}
text {
  fill: #64748b;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.chart-grid line {
  stroke: #e2e8f0;
  stroke-dasharray: 3 4;
}
.chart-line {
  fill: none;
  stroke: var(--color-primary, #2563eb);
  stroke-width: 2;
  stroke-linejoin: round;
}
.chart-point {
  fill: var(--color-primary, #2563eb);
}
.chart-point:hover {
  r: 5;
}
.chart-empty {
  color: #64748b;
  font-size: 13px;
  padding: 35px 0;
  text-align: center;
}
</style>
