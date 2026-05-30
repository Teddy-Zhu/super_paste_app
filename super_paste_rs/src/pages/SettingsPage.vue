<template>
  <div class="settings-page">
    <header class="header">
      <h1>{{ t('settings.title') }}</h1>
    </header>

    <main class="content">
      <section class="section">
        <h2>{{ t('settings.general') }}</h2>
        <div class="form-group">
          <label>{{ t('settings.maxHistoryItems') }}</label>
          <input
            v-model.number="localConfig.max_history_items"
            type="number"
            @change="handleSave"
          />
        </div>
        <div class="form-group">
          <label>{{ t('settings.autoPasteDelay') }}</label>
          <input
            v-model.number="localConfig.auto_paste_delay_ms"
            type="number"
            @change="handleSave"
          />
        </div>
        <div class="form-group">
          <label>
            <input
              v-model="localConfig.enable_ocr"
              type="checkbox"
              @change="handleSave"
            />
            {{ t('settings.enableOcr') }}
          </label>
        </div>
      </section>

      <section class="section">
        <h2>{{ t('settings.appearance') }}</h2>
        <div class="form-group">
          <label>{{ t('settings.theme') }}</label>
          <select v-model="localConfig.theme" @change="handleSave">
            <option value="auto">{{ t('settings.themes.auto') }}</option>
            <option value="light">{{ t('settings.themes.light') }}</option>
            <option value="dark">{{ t('settings.themes.dark') }}</option>
          </select>
        </div>
        <div class="form-group">
          <label>{{ t('settings.overlayPosition') }}</label>
          <select v-model="localConfig.overlay_position" @change="handleSave">
            <option value="top">{{ t('settings.positions.top') }}</option>
            <option value="bottom">{{ t('settings.positions.bottom') }}</option>
            <option value="left">{{ t('settings.positions.left') }}</option>
            <option value="right">{{ t('settings.positions.right') }}</option>
            <option value="center">{{ t('settings.positions.center') }}</option>
          </select>
        </div>
      </section>

      <section class="section">
        <h2>{{ t('settings.shortcuts') }}</h2>
        <div class="form-group">
          <label>{{ t('settings.globalShortcut') }}</label>
          <input
            v-model="localConfig.global_shortcut"
            type="text"
            @change="handleSave"
          />
        </div>
      </section>

      <section class="section">
        <h2>{{ t('settings.sync') }}</h2>
        <div class="form-group">
          <label>
            <input
              v-model="localConfig.enable_sync"
              type="checkbox"
              @change="handleSave"
            />
            {{ t('settings.enableSync') }}
          </label>
        </div>
        <div v-if="localConfig.enable_sync">
          <div class="form-group">
            <label>{{ t('settings.syncServerUrl') }}</label>
            <input
              v-model="localConfig.sync_server_url"
              type="text"
              @change="handleSave"
            />
          </div>
          <div class="form-group">
            <label>{{ t('settings.syncUsername') }}</label>
            <input
              v-model="localConfig.sync_username"
              type="text"
              @change="handleSave"
            />
          </div>
          <div class="form-group">
            <label>{{ t('settings.syncPassword') }}</label>
            <input
              v-model="localConfig.sync_password"
              type="password"
              @change="handleSave"
            />
          </div>
        </div>
      </section>
    </main>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { useI18n } from 'vue-i18n';
import { useConfigStore } from '@/stores';
import type { AppConfig } from '@/api';

const { t } = useI18n();
const configStore = useConfigStore();

const localConfig = ref<AppConfig>({ ...configStore.config });

const handleSave = async () => {
  await configStore.updateConfig(localConfig.value);
};

onMounted(async () => {
  await configStore.loadConfig();
  localConfig.value = { ...configStore.config };
});
</script>

<style scoped>
.settings-page {
  padding: 2rem;
  max-width: 900px;
  margin: 0 auto;
  height: 100vh;
  overflow-y: auto;
}

.header {
  margin-bottom: 2rem;
}

.header h1 {
  font-size: 2rem;
  font-weight: 600;
}

.section {
  margin-bottom: 2rem;
  padding: 1.5rem;
  background: white;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
}

.section h2 {
  margin-bottom: 1.5rem;
  font-size: 1.25rem;
  font-weight: 600;
  color: #333;
}

.form-group {
  margin-bottom: 1.25rem;
}

.form-group:last-child {
  margin-bottom: 0;
}

.form-group label {
  display: block;
  margin-bottom: 0.5rem;
  font-weight: 500;
  color: #555;
}

.form-group input[type='text'],
.form-group input[type='number'],
.form-group input[type='password'],
.form-group select {
  width: 100%;
  padding: 0.625rem 0.875rem;
  border: 1px solid #d0d0d0;
  border-radius: 8px;
  font-size: 14px;
  transition: border-color 0.2s;
}

.form-group input[type='text']:focus,
.form-group input[type='number']:focus,
.form-group input[type='password']:focus,
.form-group select:focus {
  outline: none;
  border-color: #007bff;
}

.form-group input[type='checkbox'] {
  margin-right: 0.5rem;
  width: 16px;
  height: 16px;
  cursor: pointer;
}

.form-group label:has(input[type='checkbox']) {
  display: flex;
  align-items: center;
  cursor: pointer;
}
</style>
