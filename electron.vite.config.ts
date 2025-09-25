import { resolve } from 'path'
import { defineConfig, externalizeDepsPlugin, bytecodePlugin } from 'electron-vite'
import vue from '@vitejs/plugin-vue'
import VueDevTools from 'vite-plugin-vue-devtools'
import MetaLayouts from 'vite-plugin-vue-meta-layouts'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import VueRouter from 'unplugin-vue-router/vite'
import Icons from 'unplugin-icons/vite'
import commonjsExternals from 'vite-plugin-commonjs-externals'

export default defineConfig({
  main: {
    plugins: [
      externalizeDepsPlugin(),
      bytecodePlugin(),
      commonjsExternals({
        externals: ['@paymoapp/active-window', '@teddyzhu/auto-launch']
      })
    ]
  },
  preload: {
    plugins: [
      externalizeDepsPlugin(),
      bytecodePlugin(),
      commonjsExternals({
        externals: ['@paymoapp/active-window', '@teddyzhu/auto-launch']
      })
    ]
  },
  renderer: {
    build: {
      rollupOptions: {
        external: ['@paymoapp/active-window', '@teddyzhu/auto-launch', 'onnxruntime-node']
      }
    },
    resolve: {
      alias: {
        '@renderer': resolve('src/renderer/src')
      }
    },
    plugins: [
      VueDevTools(),
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
        dts: './src/auto-imports.d.ts',
        dirs: ['./src/stores'],
        vueTemplate: true
      }),
      Icons({
        compiler: 'vue3',
        autoInstall: true
      }),
      Components({
        dirs: ['./src/components'],
        dts: './src/components.d.ts',
        directoryAsNamespace: true
      })
    ]
  }
})
