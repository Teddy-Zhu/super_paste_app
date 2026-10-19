import { resolve } from 'path'
import { defineConfig } from 'electron-vite'
import vue from '@vitejs/plugin-vue'
import MetaLayouts from 'vite-plugin-vue-meta-layouts'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import VueRouter from 'unplugin-vue-router/vite'
import Icons from 'unplugin-icons/vite'
import { useXWayland } from './src/main/system/session'

// Ozone 在主进程 JS 加载前初始化，开发和预览必须在启动 Electron 前传参。
if (useXWayland) {
  const args: string[] = JSON.parse(process.env.ELECTRON_CLI_ARGS || '[]')
  if (args[args.length - 1] !== '--ozone-platform=x11') {
    process.env.ELECTRON_CLI_ARGS = JSON.stringify([...args, '--ozone-platform=x11'])
  }
}

export default defineConfig({
  main: {
    build: {
      rollupOptions: {
        output: {
          // CommonJS 加载 ESM 依赖时，需要解包默认导出。
          interop: 'auto'
        }
      }
    }
  },
  preload: {},
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
