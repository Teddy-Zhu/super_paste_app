import { resolve } from 'node:path'
import process from 'node:process'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import Icons from 'unplugin-icons/vite'
import VueRouter from 'unplugin-vue-router/vite'
import MetaLayouts from 'vite-plugin-vue-meta-layouts'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const host = process.env.TAURI_DEV_HOST
const alias = {
  '@': resolve('src/renderer/src'),
  '@renderer': resolve('src/renderer/src'),
  '@shared': resolve('src/shared')
}

export default defineConfig({
  clearScreen: false,
  plugins: [
    VueRouter({
      routesFolder: './src/renderer/src/pages',
      dts: './src/renderer/src/typed-router.d.ts'
    }),
    vue(),
    MetaLayouts({
      target: './src/renderer/src/layouts',
      defaultLayout: 'default',
      skipTopLevelRouteLayout: true
    }),
    AutoImport({
      imports: ['vue', 'vue-router', 'pinia', '@vueuse/core'],
      dts: './src/renderer/src/auto-imports.d.ts',
      dirs: ['./src/renderer/src/stores'],
      vueTemplate: true
    }),
    Icons({ compiler: 'vue3' }),
    Components({
      dirs: ['./src/renderer/src/components'],
      dts: './src/renderer/src/components.d.ts',
      directoryAsNamespace: true
    })
  ],
  resolve: { alias },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] }
  }
})
