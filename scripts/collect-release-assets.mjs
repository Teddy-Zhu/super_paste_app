import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import { isCurrentInstaller, packageName, projectRoot, version } from './project.mjs'

const [target, outputArgument = 'release-assets'] = process.argv.slice(2)
if (!target) {
  throw new Error('usage: collect-release-assets.mjs <target> [output-directory]')
}

const extensionByTarget = {
  'x86_64-pc-windows-msvc': new Set(['.exe', '.msi']),
  'x86_64-apple-darwin': new Set(['.dmg']),
  'aarch64-apple-darwin': new Set(['.dmg']),
  'x86_64-unknown-linux-gnu': new Set(['.deb', '.rpm', '.AppImage'])
}
const extensions = extensionByTarget[target]
if (!extensions) throw new Error(`unsupported release target: ${target}`)

function walk(directory) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...walk(file))
    else if (entry.isFile()) files.push(file)
  }
  return files
}

const bundleRoot = path.join(projectRoot, 'target', 'release', 'bundle')
if (!existsSync(bundleRoot)) throw new Error(`Tauri bundle output does not exist: ${bundleRoot}`)
const installers = walk(bundleRoot).filter(
  (file) => extensions.has(path.extname(file)) && isCurrentInstaller(file)
)
if (installers.length === 0) throw new Error(`no installer artifacts found for ${target}`)

const migrationBinary = path.join(
  projectRoot,
  'target',
  'release',
  process.platform === 'win32' ? 'migrate-electron.exe' : 'migrate-electron'
)
if (!existsSync(migrationBinary) || !statSync(migrationBinary).isFile()) {
  throw new Error(`migration utility binary is missing: ${migrationBinary}`)
}

const ortManifest = path.join(projectRoot, 'resources', 'ort', target, 'runtime-manifest.json')
if (!existsSync(ortManifest)) throw new Error(`ORT runtime manifest is missing: ${ortManifest}`)

const output = path.resolve(outputArgument)
mkdirSync(output, { recursive: true })
const portableDirectory = path.join(projectRoot, 'target', 'release', 'portable')
const portableAssets = existsSync(portableDirectory)
  ? readdirSync(portableDirectory, { withFileTypes: true })
      .filter(
        (entry) =>
          entry.isFile() && entry.name.startsWith(`${packageName}_${version}_${target}_portable.`)
      )
      .map((entry) => ({
        source: path.join(portableDirectory, entry.name),
        name: entry.name
      }))
  : []
const assets = [
  ...installers.map((file) => ({
    source: file,
    name: `${packageName}_${version}_${target}${path.extname(file)}`
  })),
  {
    source: migrationBinary,
    name: `${packageName}-migrate-electron_${version}_${target}${process.platform === 'win32' ? '.exe' : ''}`
  },
  { source: ortManifest, name: `ort-runtime-manifest_${version}_${target}.json` },
  ...portableAssets
]

if (new Set(assets.map((asset) => asset.name)).size !== assets.length) {
  throw new Error('multiple installers map to the same release asset; separate build targets first')
}

for (const asset of assets) {
  const destination = path.join(output, asset.name)
  if (path.resolve(asset.source) !== path.resolve(destination))
    copyFileSync(asset.source, destination)
  console.log(`collected ${asset.name}`)
}
