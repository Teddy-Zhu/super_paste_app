import { createApp } from 'vue';
import { createPinia } from 'pinia';
import OverlayApp from './OverlayApp.vue';
import { i18n } from './i18n';

const app = createApp(OverlayApp);

app.use(createPinia());
app.use(i18n);

app.mount('#app');
