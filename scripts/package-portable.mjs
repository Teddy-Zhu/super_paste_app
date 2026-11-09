import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  statSync
} from 'node:fs'
import { execFileSync } from 'node:child_process'
import path from 'node:path'
import {
  binaryName,
  isCurrentInstaller,
  packageName,
  productName,
  projectRoot,
  version
} from './project.mjs'

const [target] = process.argv.slice(2)
if (!target) throw new Error('usage: package-portable.mjs <target>')

const supportedTargets = new Set(['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu'])
if (!supportedTargets.has(target))
  throw new Error(`portable archive is not configured for ${target}`)

const targetRoot = path.join(projectRoot, 'target', 'release')
const output = path.join(targetRoot, 'portable')
const isWindows = target === 'x86_64-pc-windows-msvc'
const executable = path.join(targetRoot, isWindows ? `${binaryName}.exe` : binaryName)
const migrator = path.join(targetRoot, isWindows ? 'migrate-electron.exe' : 'migrate-electron')
const readme = path.join(projectRoot, 'packaging', 'PORTABLE-README.txt')

if ((isWindows && process.platform !== 'win32') || (!isWindows && process.platform !== 'linux')) {
  throw new Error(`portable archives must be assembled on their native host (${target})`)
}
for (const file of [
  executable,
  migrator,
  readme,
  path.join(projectRoot, 'LICENSE'),
  path.join(projectRoot, 'THIRD-PARTY-NOTICES.md'),
  path.join(projectRoot, 'src-tauri/vendor/clipboard-core/LICENSE'),
  path.join(projectRoot, 'src-tauri/vendor/oar-ocr-core/LICENSE'),
  path.join(projectRoot, 'src-tauri/src/ocr/LAYOUT-NOTICE.md')
]) {
  if (!existsSync(file) || !statSync(file).isFile())
    throw new Error(`portable input is missing: ${file}`)
}

mkdirSync(output, { recursive: true })
const stagingRoot = mkdtempSync(path.join(output, '.stage-'))
const stage = path.join(stagingRoot, productName)
mkdirSync(stage, { recursive: true })
try {
  if (isWindows) {
    copyFileSync(executable, path.join(stage, `${binaryName}.exe`))
    const resources = path.join(stage, 'resources')
    mkdirSync(resources, { recursive: true })
    copyFileSync(path.join(projectRoot, 'LICENSE'), path.join(resources, 'LICENSE'))
    copyFileSync(
      path.join(projectRoot, 'THIRD-PARTY-NOTICES.md'),
      path.join(resources, 'THIRD-PARTY-NOTICES.md')
    )
    const licenseDirectory = path.join(resources, 'licenses')
    mkdirSync(licenseDirectory, { recursive: true })
    copyFileSync(
      path.join(projectRoot, 'src-tauri/vendor/clipboard-core/LICENSE'),
      path.join(licenseDirectory, 'MIT-clipboard-core.txt')
    )
    copyFileSync(
      path.join(projectRoot, 'src-tauri/vendor/oar-ocr-core/LICENSE'),
      path.join(licenseDirectory, 'Apache-2.0.txt')
    )
    copyFileSync(
      path.join(projectRoot, 'src-tauri/src/ocr/LAYOUT-NOTICE.md'),
      path.join(licenseDirectory, 'LAYOUT-NOTICE.md')
    )
    copyFileSync(
      path.join(projectRoot, 'resources', 'ocr-models.json'),
      path.join(resources, 'ocr-models.json')
    )
    const ortSource = path.join(projectRoot, 'resources', 'ort', target)
    if (!existsSync(ortSource)) throw new Error(`ORT resources are missing: ${ortSource}`)
    cpSync(ortSource, path.join(resources, 'ort', target), { recursive: true })
  } else {
    const appImages = []
    const pending = [path.join(targetRoot, 'bundle')]
    while (pending.length) {
      const current = pending.pop()
      if (!existsSync(current)) continue
      for (const entry of readdirSync(current, { withFileTypes: true })) {
        const file = path.join(current, entry.name)
        if (entry.isDirectory()) pending.push(file)
        else if (entry.isFile() && path.extname(file) === '.AppImage' && isCurrentInstaller(file))
          appImages.push(file)
      }
    }
    if (appImages.length !== 1)
      throw new Error(
        `expected one Linux AppImage for the portable archive; found ${appImages.length}`
      )
    const appImage = path.join(stage, 'Super-Paste.AppImage')
    copyFileSync(appImages[0], appImage)
    chmodSync(appImage, 0o755)
  }

  copyFileSync(migrator, path.join(stage, isWindows ? 'migrate-electron.exe' : 'migrate-electron'))
  copyFileSync(path.join(projectRoot, 'LICENSE'), path.join(stage, 'LICENSE'))
  copyFileSync(
    path.join(projectRoot, 'THIRD-PARTY-NOTICES.md'),
    path.join(stage, 'THIRD-PARTY-NOTICES.md')
  )
  copyFileSync(readme, path.join(stage, 'README.txt'))
  mkdirSync(output, { recursive: true })

  const archive = path.join(
    output,
    `${packageName}_${version}_${target}_portable.${isWindows ? 'zip' : 'tar.gz'}`
  )
  if (isWindows) {
    execFileSync(
      'powershell.exe',
      [
        '-NoLogo',
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        "Compress-Archive -Path (Join-Path $args[0] '*') -DestinationPath $args[1] -Force",
        stage,
        archive
      ],
      { stdio: 'inherit' }
    )
  } else {
    execFileSync('tar', ['-czf', archive, '-C', stage, '.'], { stdio: 'inherit' })
  }
  console.log(`created ${archive}`)
} finally {
  rmSync(stagingRoot, { recursive: true, force: true })
}
