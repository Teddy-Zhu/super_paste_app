<template>
  <SettingsPage />
</template>

<script setup lang="ts">
import { onMounted } from 'vue';
import { listen } from '@tauri-apps/api/event';
import SettingsPage from '@/pages/SettingsPage.vue';

onMounted(async () => {
  // 监听快捷键变更事件
  await listen('shortcut:changed', (event) => {
    console.log('快捷键已变更:', event.payload);
  });
});
</script>

<style>
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial,
    sans-serif;
  font-size: 14px;
  line-height: 1.5;
  color: #333;
  background: #f5f5f5;
}

#app {
  width: 100%;
  height: 100vh;
}

button {
  font-family: inherit;
}

input,
select,
textarea {
  font-family: inherit;
  font-size: inherit;
}
</style>