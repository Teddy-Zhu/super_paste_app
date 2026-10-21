import { dirname, join, resolve } from 'node:path'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { defineConfig } from 'electron-vite'
import type { Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import MetaLayouts from 'vite-plugin-vue-meta-layouts'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import VueRouter from 'unplugin-vue-router/vite'
import Icons from 'unplugin-icons/vite'
import { useXWayland } from './src/main/system/session'

function bundledLicenses(): Plugin {
  return {
    name: 'bundled-dependency-licenses',
    generateBundle(_options, bundle) {
      const packages = new Set<string>()
      for (const id of this.getModuleIds()) {
        if (
          id.startsWith('\0') ||
          !id.includes('/node_modules/') ||
          this.getModuleInfo(id)?.isExternal
        )
          continue
        let directory = dirname(id.split('?')[0])
        while (directory.includes('/node_modules/')) {
          const manifest = join(directory, 'package.json')
          if (existsSync(manifest) && JSON.parse(readFileSync(manifest, 'utf8')).name) {
            packages.add(directory)
            break
          }
          directory = dirname(directory)
        }
      }
      const notices = [...packages].sort().map((directory) => {
        const manifest = JSON.parse(readFileSync(join(directory, 'package.json'), 'utf8'))
        const licenses = readdirSync(directory)
          .filter((name) => /^(licen[cs]e|copying|notice)(?:[.-]|$)/i.test(name))
          .filter((name) => statSync(join(directory, name)).isFile())
          .sort()
          .map((name) => readFileSync(join(directory, name), 'utf8'))
        return `${manifest.name}@${manifest.version} (${manifest.license || 'see upstream'})\n${licenses.join('\n')}`
      })
      if (!notices.length) return
      // Worker 构建也会执行此插件，按入口命名以免互相覆盖许可证。
      const entry = Object.values(bundle)
        .filter((output) => output.type === 'chunk' && output.isEntry)
        .map((output) => output.name)
        .sort()
        .join('-')
      this.emitFile({
        type: 'asset',
        fileName: `THIRD_PARTY_LICENSES-${entry}.txt`,
        source: notices.join('\n\n----------------------------------------\n\n')
      })
    }
  }
}

const alias = {
  '@': resolve('src/renderer/src'),
  '@renderer': resolve('src/renderer/src'),
  '@shared': resolve('src/shared'),
  '@main': resolve('src/main'),
  '@preload': resolve('src/preload')
}

// Ozone 在主进程 JS 加载前初始化，开发和预览必须在启动 Electron 前传参。
if (useXWayland) {
  const args: string[] = JSON.parse(process.env.ELECTRON_CLI_ARGS || '[]')
  if (args[args.length - 1] !== '--ozone-platform=x11') {
    process.env.ELECTRON_CLI_ARGS = JSON.stringify([...args, '--ozone-platform=x11'])
  }
}

export default defineConfig({
  main: {
    resolve: { alias },
    plugins: [bundledLicenses()],
    build: {
      // 纯 JS 依赖放在 devDependencies，内联后不再重复复制整包。
      minify: 'esbuild',
      rollupOptions: {
        output: {
          // CommonJS 加载 ESM 依赖时，需要解包默认导出。
          interop: 'auto'
        }
      }
    }
  },
  preload: { resolve: { alias } },
  renderer: {
    build: {
      rollupOptions: {
        external: ['@paymoapp/active-window', '@teddyzhu/auto-launch', 'onnxruntime-node']
      }
    },
    resolve: { alias },
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
