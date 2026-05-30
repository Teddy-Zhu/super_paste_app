<template>
  <div class="clipboard-item-card" :class="{ pinned: item.is_pinned, favorite: item.is_favorite }">
    <div class="card-header">
      <span class="type-badge">{{ getTypeLabel(item.primary_type) }}</span>
      <div class="actions">
        <button
          @click="$emit('toggle-pin', item.id)"
          class="btn-icon"
          :title="item.is_pinned ? t('clipboard.unpin') : t('clipboard.pin')"
        >
          📌
        </button>
        <button
          @click="$emit('toggle-favorite', item.id)"
          class="btn-icon"
          :title="item.is_favorite ? t('clipboard.unfavorite') : t('clipboard.favorite')"
        >
          {{ item.is_favorite ? '⭐' : '☆' }}
        </button>
        <button
          @click="handleCopy"
          class="btn-icon"
          title="Copy"
        >
          📋
        </button>
        <button
          @click="$emit('delete', item.id)"
          class="btn-icon btn-delete"
          :title="t('common.delete')"
        >
          🗑️
        </button>
      </div>
    </div>

    <div class="card-content">
      <div v-if="item.preview_text" class="preview-text">
        {{ item.preview_text }}
      </div>
      <div v-else class="preview-placeholder">
        {{ getTypeLabel(item.primary_type) }}
      </div>
    </div>

    <div class="card-footer">
      <span class="timestamp">{{ formatTime(item.last_used) }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { useClipboard } from '@/composables/useUtils';
import type { ClipboardItem } from '@/api';

interface Props {
  item: ClipboardItem;
}

const props = defineProps<Props>();
defineEmits<{
  delete: [id: number];
  'toggle-pin': [id: number];
  'toggle-favorite': [id: number];
}>();

const { t } = useI18n();
const { copy } = useClipboard();

const getTypeLabel = (type: string): string => {
  const typeMap: Record<string, string> = {
    text: t('clipboard.types.text'),
    image: t('clipboard.types.image'),
    file: t('clipboard.types.file'),
    html: t('clipboard.types.html'),
    rtf: t('clipboard.types.rtf'),
  };
  return typeMap[type] || type;
};

const formatTime = (timestamp: string): string => {
  const date = new Date(timestamp);
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(diff / 3600000);
  const days = Math.floor(diff / 86400000);

  if (minutes < 1) return '刚刚';
  if (minutes < 60) return `${minutes}分钟前`;
  if (hours < 24) return `${hours}小时前`;
  if (days < 7) return `${days}天前`;
  return date.toLocaleDateString();
};

const handleCopy = async () => {
  if (props.item.preview_text) {
    const success = await copy(props.item.preview_text);
    if (success) {
      console.log(t('clipboard.copySuccess'));
    } else {
      console.error(t('clipboard.copyFailed'));
    }
  }
};
</script>

<style scoped>
.clipboard-item-card {
  padding: 1rem;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  background: white;
  transition: all 0.2s;
}

.clipboard-item-card:hover {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}

.clipboard-item-card.pinned {
  border-left: 4px solid #007bff;
}

.clipboard-item-card.favorite {
  background: #fffbf0;
}

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.5rem;
}

.type-badge {
  padding: 0.25rem 0.5rem;
  background: #e9ecef;
  border-radius: 4px;
  font-size: 0.875rem;
  font-weight: 500;
}

.actions {
  display: flex;
  gap: 0.25rem;
}

.btn-icon {
  padding: 0.25rem 0.5rem;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 1rem;
  opacity: 0.6;
  transition: opacity 0.2s;
}

.btn-icon:hover {
  opacity: 1;
}

.btn-delete:hover {
  color: #dc3545;
}

.card-content {
  margin-bottom: 0.5rem;
}

.preview-text {
  max-height: 4rem;
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  line-height: 1.4;
  color: #333;
}

.preview-placeholder {
  color: #999;
  font-style: italic;
}

.card-footer {
  display: flex;
  justify-content: flex-end;
}

.timestamp {
  font-size: 0.75rem;
  color: #999;
}
</style>
