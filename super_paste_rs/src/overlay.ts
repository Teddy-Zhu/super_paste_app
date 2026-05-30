import { createApp } from 'vue';
import OverlayApp from './OverlayApp.vue';
import { i18n } from './i18n';
import store from '@renderer/stores';
import './assets/main.css';

const app = createApp(OverlayApp);

app.use(store);
app.use(i18n);

app.mount('#app');
