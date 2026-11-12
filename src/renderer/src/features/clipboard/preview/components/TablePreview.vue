<script setup lang="ts">
import { computed } from 'vue'
import { PREVIEW_LABELS, PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import type { PreviewProps, PreviewEvents } from '../types'

const props = withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
const visibleRows = computed(() =>
  props.expanded
    ? props.preview.table?.rows
    : props.preview.table?.rows.slice(0, PREVIEW_LIMITS.tableRows)
)
</script>

<template>
  <template v-if="preview.table">
    <div class="table-wrapper" :class="{ expanded }">
      <table class="data-table" :class="{ expanded }">
        <tbody>
          <tr v-for="(row, rowIndex) in visibleRows" :key="rowIndex">
            <th scope="row">{{ rowIndex + 1 }}</th>
            <td
              v-for="(cell, columnIndex) in expanded
                ? row
                : row.slice(0, PREVIEW_LIMITS.tableColumns)"
              :key="columnIndex"
              :title="cell"
              dir="auto"
            >
              {{ cell }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p
      v-if="
        !expanded &&
        (preview.table.rows.length > PREVIEW_LIMITS.tableRows ||
          preview.table.columns > PREVIEW_LIMITS.tableColumns)
      "
      class="preview-notice"
    >
      {{ PREVIEW_LABELS.previewExcerpt }}
    </p>
  </template>
</template>

<style scoped>
.data-table {
  border-collapse: collapse;
  font-size: 12px;
  text-align: start;
}
.data-table td,
.data-table th {
  border: 1px solid var(--color-control-border);
  padding: 5px 8px;
  vertical-align: top;
}
.table-wrapper {
  overflow: hidden;
  min-height: 0;
}
.data-table {
  width: 100%;
  table-layout: fixed;
}
.data-table th {
  width: 28px;
  padding-inline: 3px;
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
  font-size: 10px;
  text-align: center;
}
.data-table td {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 100px;
}
.data-table tr:nth-child(even) td {
  background: var(--color-bg-tertiary);
}
.table-wrapper.expanded {
  overflow: visible;
}
.data-table.expanded {
  table-layout: auto;
}
.data-table.expanded td {
  min-width: 100px;
}
</style>
