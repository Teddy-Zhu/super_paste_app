<template>
  <div class="home-page">
    <header class="header">
      <h1>{{ t('common.appName') }}</h1>
      <div class="actions">
        <input
          v-model="searchQuery"
          type="text"
          :placeholder="t('common.search')"
          class="search-input"
          @input="handleSearch"
        />
        <button @click="goToSettings" class="btn-settings">
          {{ t('common.settings') }}
        </button>
      </div>
    </header>

    <main class="content">
      <div v-if="clipboardStore.loading" class="loading">
        {{ t('common.loading') }}
      </div>

      <div v-else-if="clipboardStore.items.length === 0" class="empty">
        {{ t('clipboard.empty') }}
      </div>

      <div v-else class="clipboard-list">
        <ClipboardItemCard
          v-for="item in clipboardStore.items"
          :key="item.id"
          :item="item"
          @delete="handleDelete"
          @toggle-pin="handleTogglePin"
          @toggle-favorite="handleToggleFavorite"
        />
      </div>
    </main>

    <footer class="footer">
      <span>{{ t('clipboard.history') }}: {{ clipboardStore.totalCount }}</span>
      <button @click="handleClearHistory" class="btn-clear">
        {{ t('clipboard.clear') }}
      </button>
    </footer>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useClipboardStore } from '@/stores';
import { useDebounce } from '@/composables/useUtils';
import ClipboardItemCard from '@/components/clipboard/ClipboardItemCard.vue';

const { t } = useI18n();
const router = useRouter();
const clipboardStore = useClipboardStore();

const searchQuery = ref('');

const handleSearch = useDebounce(async () => {
  if (searchQuery.value.trim()) {
    await clipboardStore.search(searchQuery.value);
  } else {
    await clipboardStore.fetchHistory();
  }
}, 300);

const handleDelete = async (id: number) => {
  if (confirm(t('clipboard.deleteConfirm'))) {
    await clipboardStore.deleteItem(id);
  }
};

const handleTogglePin = async (id: number) => {
  await clipboardStore.togglePin(id);
};

const handleToggleFavorite = async (id: number) => {
  await clipboardStore.toggleFavorite(id);
};

const handleClearHistory = async () => {
  if (confirm(t('clipboard.clearConfirm'))) {
    await clipboardStore.clearHistory();
  }
};

const goToSettings = () => {
  router.push('/settings');
};

onMounted(async () => {
  await clipboardStore.fetchHistory();
});
</script>

<style scoped>
.home-page {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 1rem;
}

.header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.actions {
  display: flex;
  gap: 0.5rem;
}

.search-input {
  padding: 0.5rem;
  border: 1px solid #ccc;
  border-radius: 4px;
  min-width: 200px;
}

.btn-settings,
.btn-clear {
  padding: 0.5rem 1rem;
  border: none;
  border-radius: 4px;
  background: #007bff;
  color: white;
  cursor: pointer;
}

.btn-settings:hover,
.btn-clear:hover {
  background: #0056b3;
}

.content {
  flex: 1;
  overflow-y: auto;
}

.loading,
.empty {
  text-align: center;
  padding: 2rem;
  color: #666;
}

.clipboard-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.footer {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid #eee;
}
</style>
