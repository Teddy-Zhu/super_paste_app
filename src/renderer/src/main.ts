import './assets/main.css'

import { createApp } from 'vue'
import { initializeDesktop } from './api/desktop'
import { applyColorScheme } from './utils/color-mode'

async function start(): Promise<void> {
  const bootstrap = await initializeDesktop()
  applyColorScheme(
    bootstrap.config.settings.snapshot.value?.colorMode,
    window.matchMedia('(prefers-color-scheme: dark)').matches
  )
  const [{ default: router }, { default: store }, { default: App }] = await Promise.all([
    import('./router'),
    import('./stores'),
    import('./App.vue')
  ])

  const app = createApp(App)
  app.use(router)
  app.use(store)
  app.mount('#app')
}

void start().catch((error) => {
  console.error('桌面配置初始化失败:', error)
  const root = document.querySelector('#app')
  if (root) {
    root.textContent = `应用初始化失败：${error instanceof Error ? error.message : String(error)}`
  }
})
