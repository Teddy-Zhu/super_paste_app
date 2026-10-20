<script setup lang="ts">
import { computed } from 'vue'
import { PREVIEW_LABELS, PREVIEW_LIMITS } from '@renderer/constants/clipboard-preview'
import type { ClipboardPreview } from '@renderer/utils/clipboard-preview'
import { previewExcerpt } from '@renderer/utils/clipboard-preview'
import IconFile from '~icons/mdi/file-outline'
import IconLink from '~icons/mdi/link-variant'
import IconAt from '~icons/mdi/at'
import IconPhone from '~icons/mdi/phone-outline'
import ImageDisplay from './ImageDisplay.vue'

const props = withDefaults(
  defineProps<{
    preview: ClipboardPreview
    expanded?: boolean
    plain?: boolean
  }>(),
  { expanded: false, plain: false }
)

const emit = defineEmits<{ imageLoad: [event: Event] }>()
const text = computed(() => (props.plain ? props.preview.text : props.preview.displayText))
const displayedText = computed(() => {
  const value = props.expanded ? text.value : previewExcerpt(text.value, PREVIEW_LIMITS.text)
  return value && !value.trim()
    ? value
        .replace(/ /g, '·')
        .replace(/\t/g, '→   ')
        .replace(/\r\n|\r|\n/g, '↵\n')
    : value
})
const visibleFiles = computed(() =>
  props.expanded ? props.preview.files : props.preview.files.slice(0, PREVIEW_LIMITS.fileRows)
)
const visibleRows = computed(() =>
  props.expanded
    ? props.preview.table?.rows
    : props.preview.table?.rows.slice(0, PREVIEW_LIMITS.tableRows)
)
const compactText = computed(() => text.value.length <= 80 && !/[\r\n]/.test(text.value))
</script>

<template>
  <div class="preview-content" :class="[{ expanded, 'plain-mode': plain }, `kind-${preview.kind}`]">
    <p v-if="preview.notice" class="preview-notice">{{ preview.notice }}</p>

    <template v-if="preview.kind === 'files'">
      <div class="file-list">
        <div v-for="file in visibleFiles" :key="file.path" class="file-row" :title="file.path">
          <div class="file-symbol">
            <IconFile /><span>{{ file.extension }}</span>
          </div>
          <div class="file-info">
            <div class="file-name" dir="auto">{{ file.name }}</div>
            <div v-if="file.directory" class="file-directory" dir="auto">{{ file.directory }}</div>
          </div>
        </div>
      </div>
      <p v-if="visibleFiles.length < preview.files.length" class="preview-notice">
        +{{ preview.files.length - visibleFiles.length }} 个文件
      </p>
    </template>

    <template v-else-if="preview.kind === 'image' && preview.image">
      <div class="image-frame checkerboard">
        <ImageDisplay
          :image-data="preview.image"
          :alt="PREVIEW_LABELS.image"
          image-class="preview-image"
          @load="emit('imageLoad', $event)"
        />
      </div>
      <div v-if="preview.text.trim()" class="image-caption">
        <span class="caption-label">{{ PREVIEW_LABELS.caption }}</span>
        <p dir="auto">
          {{ expanded ? preview.text : previewExcerpt(preview.text, 300) }}
        </p>
      </div>
      <div v-if="preview.files.length" class="image-filename" :title="preview.files[0].path">
        <IconFile />{{ preview.files[0].name }}
      </div>
    </template>

    <div
      v-else-if="preview.kind === 'rich' && !plain"
      class="rich-content"
      v-html="preview.html"
    ></div>

    <template v-else-if="preview.kind === 'table' && preview.table && !plain">
      <div class="table-wrapper">
        <table class="data-table">
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

    <template v-else-if="preview.kind === 'url' && preview.url">
      <div class="identity-heading">
        <IconLink /><strong>{{ preview.url.host }}</strong>
      </div>
      <p class="url-path" dir="auto">
        {{
          expanded ? preview.url.detail : previewExcerpt(preview.url.detail, PREVIEW_LIMITS.text)
        }}
      </p>
      <p class="original-url" dir="auto">
        {{ expanded ? preview.url.href : previewExcerpt(preview.url.href, PREVIEW_LIMITS.text) }}
      </p>
    </template>

    <template v-else-if="preview.kind === 'email' || preview.kind === 'phone'">
      <div class="contact-icon"><IconAt v-if="preview.kind === 'email'" /><IconPhone v-else /></div>
      <p class="contact-value" dir="auto">{{ preview.text.trim() }}</p>
    </template>

    <template v-else-if="preview.kind === 'color' && preview.color">
      <div class="color-swatch checkerboard">
        <div :style="{ backgroundColor: preview.color }"></div>
      </div>
      <code class="color-value">{{ preview.color }}</code>
    </template>

    <template v-else-if="text">
      <span v-if="!text.trim()" class="preview-notice">{{ PREVIEW_LABELS.whitespace }}</span>
      <pre
        class="text-preview"
        :class="{ 'code-preview': preview.kind === 'code', 'compact-text': compactText }"
        dir="auto"
        >{{ displayedText }}</pre
      >
      <span v-if="!expanded && text.length > PREVIEW_LIMITS.text" class="preview-notice">{{
        PREVIEW_LABELS.previewExcerpt
      }}</span>
    </template>

    <div v-else class="empty-preview">{{ PREVIEW_LABELS.empty }}</div>
  </div>
</template>

<style scoped>
.preview-content {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
  color: #334155;
  font-size: 14px;
  line-height: 1.65;
}
.preview-notice {
  margin: 0;
  color: #64748b;
  font-size: 11px;
  flex-shrink: 0;
}
.text-preview {
  margin: 0;
  min-height: 0;
  overflow: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: inherit;
  tab-size: 4;
}
.compact-text {
  font-size: 22px;
  line-height: 1.5;
  font-weight: 500;
  margin-block: auto;
}
.code-preview {
  font-family: var(--font-family-mono, monospace);
  font-size: 12px;
  line-height: 1.7;
  padding: 10px;
  background: #f1f5f9;
  border-radius: 8px;
}
.file-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  overflow: hidden;
}
.file-row {
  display: flex;
  gap: 10px;
  align-items: center;
  min-width: 0;
  flex-shrink: 0;
  padding: 8px 0;
}
.file-symbol {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
  color: #0891b2;
  width: 40px;
}
.file-symbol svg {
  width: 28px;
  height: 28px;
}
.file-symbol span {
  font-size: 9px;
  line-height: 1.3;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-info {
  min-width: 0;
}
.file-name {
  font-size: 14px;
  font-weight: 600;
  overflow-wrap: anywhere;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.file-directory {
  font-size: 11px;
  color: #64748b;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.checkerboard {
  background-color: #fff;
  background-image: conic-gradient(#e2e8f0 25%, transparent 0 50%, #e2e8f0 0 75%, transparent 0);
  background-size: 16px 16px;
}
.image-frame {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.image-frame :deep(.preview-image) {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
}
.image-caption {
  flex-shrink: 0;
  min-width: 0;
  border-top: 1px solid #e2e8f0;
  padding-top: 6px;
  max-height: 35%;
  overflow: hidden;
}
.caption-label {
  color: #64748b;
  font-size: 10px;
  font-weight: 600;
}
.image-caption p {
  margin: 2px 0 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.image-filename {
  display: flex;
  align-items: center;
  gap: 4px;
  color: #64748b;
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 0;
}
.image-filename svg {
  flex-shrink: 0;
}
.rich-content {
  min-height: 0;
  overflow: hidden;
  overflow-wrap: anywhere;
  isolation: isolate;
}
.preview-content:not(.expanded) .rich-content {
  pointer-events: none;
}
.rich-content :deep(*) {
  box-sizing: border-box;
  max-width: 100%;
}
.rich-content :deep(> :first-child) {
  margin-top: 0;
}
.rich-content :deep(p),
.rich-content :deep(blockquote),
.rich-content :deep(ul),
.rich-content :deep(ol) {
  margin-block: 0.4em;
}
.rich-content :deep(ul),
.rich-content :deep(ol) {
  padding-left: 1.5em;
}
.rich-content :deep(h1),
.rich-content :deep(h2),
.rich-content :deep(h3) {
  font-size: 1.15em;
  margin-block: 0.5em;
}
.rich-content :deep(pre),
.rich-content :deep(code) {
  font-family: var(--font-family-mono, monospace);
  white-space: pre-wrap;
}
.rich-content :deep(img) {
  height: auto;
  object-fit: contain;
}
.rich-content :deep(table),
.data-table {
  border-collapse: collapse;
  font-size: 12px;
  text-align: start;
}
.rich-content :deep(table) {
  table-layout: fixed;
  width: 100%;
}
.rich-content :deep(td),
.rich-content :deep(th),
.data-table td,
.data-table th {
  border: 1px solid #cbd5e1;
  padding: 5px 8px;
  vertical-align: top;
}
.rich-content :deep(blockquote) {
  border-left: 3px solid #cbd5e1;
  padding-left: 10px;
  color: #64748b;
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
  color: #64748b;
  background: #f1f5f9;
  font-size: 10px;
  text-align: center;
}
.data-table td {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 100px;
}
.data-table tr:nth-child(even) td {
  background: #f8fafc;
}
.identity-heading {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  color: #b45309;
  font-size: 18px;
  overflow-wrap: anywhere;
}
.identity-heading svg {
  flex-shrink: 0;
  margin-top: 5px;
}
.url-path {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 14px;
}
.original-url {
  margin: auto 0 0;
  font-size: 11px;
  color: #64748b;
  overflow-wrap: anywhere;
}
.contact-icon {
  font-size: 40px;
  color: #e11d48;
  margin: auto auto 0;
}
.contact-value {
  font-size: 21px;
  font-weight: 500;
  margin: 0 0 auto;
  text-align: center;
  overflow-wrap: anywhere;
}
.color-swatch {
  flex: 1;
  min-height: 48px;
  border: 1px solid #e2e8f0;
  border-radius: 10px;
  overflow: hidden;
}
.color-swatch > div {
  height: 100%;
}
.color-value {
  font-family: var(--font-family-mono, monospace);
  font-size: 18px;
  text-align: center;
  overflow-wrap: anywhere;
}
.empty-preview {
  margin: auto;
  color: #94a3b8;
  font-size: 13px;
  text-align: center;
}
.expanded {
  height: auto;
  min-height: 100%;
  overflow: visible;
  gap: 16px;
}
.expanded .text-preview {
  overflow: visible;
  font-size: 14px;
  margin: 0;
}
.expanded .code-preview {
  white-space: pre;
  min-width: max-content;
}
.expanded .file-list {
  overflow: visible;
}
.expanded .file-name {
  display: block;
}
.expanded .file-directory {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
.expanded .image-frame {
  flex: none;
  min-height: 100px;
  overflow: visible;
}
.expanded .image-frame :deep(.preview-image) {
  width: auto;
  height: auto;
  max-width: 100%;
}
.expanded .image-caption {
  max-height: none;
  overflow: visible;
}
.expanded .image-caption p {
  display: block;
}
.expanded .rich-content,
.expanded .table-wrapper {
  overflow: visible;
}
.expanded .data-table {
  table-layout: auto;
}
.expanded .data-table td {
  min-width: 100px;
}
.expanded .contact-value {
  margin: 0;
}
.expanded .color-swatch {
  flex: none;
  height: 220px;
}
</style>
