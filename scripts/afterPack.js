module.exports = async function (context) {
  const fs = await import('node:fs')
  const path = await import('node:path')
  const resourcesDir = context.packager.getResourcesDir(context.appOutDir)
  const unpackedDir = path.join(resourcesDir, 'app.asar.unpacked')
  const napiBinBase = path.join(unpackedDir, 'node_modules', 'onnxruntime-node', 'bin', 'napi-v6')
  const targetPlatform = context.electronPlatformName
  const archMap = {
    0: 'ia32',
    1: 'x64',
    2: 'armv7l',
    3: 'arm64'
  }

  const targetArch = archMap[context.arch]
  if (!targetArch) throw new Error(`[afterPack] Unsupported target architecture: ${context.arch}`)

  // 裁剪打包文件后仍须保留各模块的原生入口，缺失时中止打包。
  // eslint-disable-next-line @typescript-eslint/explicit-function-return-type -- CommonJS 钩子无法使用 TypeScript 返回类型标注。
  const requireBinary = (relativePath) => {
    if (!fs.existsSync(path.join(unpackedDir, relativePath))) {
      throw new Error(`[afterPack] Missing native module: ${relativePath}`)
    }
  }
  requireBinary('node_modules/better-sqlite3/build/Release/better_sqlite3.node')
  requireBinary('node_modules/canvas/build/Release/canvas.node')
  requireBinary('node_modules/@paymoapp/active-window/build/Release/PaymoActiveWindow.node')

  const clipboardSuffix =
    targetPlatform === 'linux' ? '-gnu' : targetPlatform === 'win32' ? '-msvc' : ''
  const clipboardTarget = `${targetPlatform}-${targetArch}${clipboardSuffix}`
  requireBinary(
    path.join(
      'node_modules',
      '@teddyzhu',
      `clipboard-${clipboardTarget}`,
      `clipboard.${clipboardTarget}.node`
    )
  )

  if (targetPlatform === 'linux') {
    const kwinBinary = `active_window_kwin.linux-${targetArch}-gnu.node`
    requireBinary(path.join('node_modules', '@super-paste', 'active-window-kwin', kwinBinary))
  }

  requireBinary(
    path.join(
      'node_modules',
      'onnxruntime-node',
      'bin',
      'napi-v6',
      targetPlatform,
      targetArch,
      'onnxruntime_binding.node'
    )
  )

  console.log(`[afterPack] Keeping only napi-v6/${targetPlatform}/${targetArch} binaries`)

  const platforms = fs.readdirSync(napiBinBase)
  for (const platform of platforms) {
    const platformPath = path.join(napiBinBase, platform)
    if (!fs.statSync(platformPath).isDirectory()) continue

    const archs = fs.readdirSync(platformPath)
    for (const arch of archs) {
      if (platform !== targetPlatform || arch !== targetArch) {
        const fullPath = path.join(platformPath, arch)
        console.log(`[afterPack] Removing unused onnxruntime binary: ${fullPath}`)
        fs.rmSync(fullPath, { recursive: true, force: true })
      }
    }

    const remaining = fs.readdirSync(platformPath)
    if (remaining.length === 0) {
      console.log(`[afterPack] Removing empty platform folder: ${platformPath}`)
      fs.rmSync(platformPath, { recursive: true, force: true })
    }
  }

  // 发布容器设置兼容基线；系统 Electron 的 AUR 源码包使用本机系统库。
  if (targetPlatform === 'linux' && process.env.SUPER_PASTE_GLIBC_MAX) {
    const { execFileSync } = await import('node:child_process')
    execFileSync(
      process.execPath,
      [path.join(__dirname, 'check-linux-compatibility.mjs'), context.appOutDir],
      { stdio: 'inherit' }
    )
  }
}
