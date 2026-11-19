import { spawnSync } from 'node:child_process'
import { cpSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { stageDependencyNotices } from './package-notices.mjs'
import { readRuntimeManifest } from './ort-manifest.mjs'
import { sha256File as sha256 } from './file-hash.mjs'
import { appImageToolEnv, compressDeb, packageAppImage, stageLinuxDesktop, stageLinuxExecutable } from './package-linux.mjs'

import { buildTools, binaryName, hostTarget, packaging, packageFormats, releaseTargets, version as workspaceVersion } from './project.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packagerVersion = buildTools.cargoPackager

function run(command, args, { capture = false, env = {}, cwd = root } = {}) {
  const result = spawnSync(command, args, {
    cwd, shell: false,
    env: { ...process.env, CARGO_TARGET_DIR: path.join(root, 'target'), ...env },
    stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit',
    encoding: 'utf8', maxBuffer: 32 * 1024 * 1024
  })
  if (result.error) throw new Error(`Could not run ${command}: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`${command} failed (${result.signal ?? result.status})`)
  return result.stdout ?? ''
}


function regularFiles(directory, prefix = '') {
  const result = []
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name
    if (entry.isDirectory()) result.push(...regularFiles(path.join(directory, entry.name), relative))
    else if (entry.isFile()) result.push(relative)
    else throw new Error(`Package staging contains a non-regular file: ${relative}`)
  }
  return result
}

function stageRuntime(target, destination) {
  const source = path.join(root, 'resources', 'ort', target)
  const spec = JSON.parse(readFileSync(path.join(root, 'resources', 'ort-runtime.json'), 'utf8'))
  const { bytes: manifestBytes, entries } = readRuntimeManifest(path.join(source, 'runtime-manifest.json'), spec, target)
  mkdirSync(destination, { recursive: true })
  for (const entry of entries) {
    const staged = path.join(destination, entry.file)
    copyFileSync(path.join(source, entry.file), staged)
    if (sha256(staged).toLowerCase() !== entry.sha256.toLowerCase()) throw new Error(`ORT checksum mismatch: ${entry.file}`)
  }
  writeFileSync(path.join(destination, 'runtime-manifest.json'), manifestBytes)
}

function stageResources(target, directory) {
  mkdirSync(directory, { recursive: true })
  stageRuntime(target, path.join(directory, 'ort', target))
  for (const file of ['ort-runtime.json', 'ocr-models.json']) copyFileSync(path.join(root, 'resources', file), path.join(directory, file))
  for (const file of ['LICENSE', 'THIRD-PARTY-NOTICES.md', 'Cargo.lock']) copyFileSync(path.join(root, file), path.join(directory, file))
  cpSync(path.join(root, 'licenses'), path.join(directory, 'licenses'), { recursive: true })
  const resourceNotices = path.join(directory, 'licenses', 'resources')
  mkdirSync(resourceNotices, { recursive: true })
  copyFileSync(path.join(root, 'resources', 'icons', 'NOTICE.md'), path.join(resourceNotices, 'icons-NOTICE.md'))
}

function portable(stage, payload, outDir, target, version) {
  const name = `${binaryName}-${version}-${target}`
  const directory = path.join(stage, name)
  mkdirSync(directory)
  copyFileSync(payload, path.join(directory, path.basename(payload)))
  cpSync(path.join(stage, 'resources'), path.join(directory, 'resources'), { recursive: true })
  if (process.platform === 'win32') {
    run('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command',
      'Compress-Archive -LiteralPath $env:SUPER_PASTE_ZIP_SOURCE -DestinationPath $env:SUPER_PASTE_ZIP_DESTINATION -Force -ErrorAction Stop'], {
      env: { SUPER_PASTE_ZIP_SOURCE: directory, SUPER_PASTE_ZIP_DESTINATION: path.join(outDir, `${name}.zip`) }
    })
  } else {
    const archive = `${name}.tar.xz`
    run('tar', ['-cJf', path.join(stage, archive), '-C', stage, name], { env: { XZ_OPT: '-6 -T2' } })
    renameSync(path.join(stage, archive), path.join(outDir, archive))
  }
}

function pacman(stage, outDir, version) {
  const directory = path.join(stage, 'pacman')
  mkdirSync(directory)
  const recipePath = path.join(outDir, 'PKGBUILD')
  let recipe = readFileSync(recipePath, 'utf8')
  // These are prebuilt application resources. makepkg's default stripping
  // would mutate the ORT libraries after their manifest hashes were recorded.
  const copyPayload = '\tcp -r "${srcdir}"/* "${pkgdir}"/'
  if (recipe.split('options=(!lto)').length !== 2 || recipe.split('pkgrel=1\n').length !== 2 || recipe.split(copyPayload).length !== 2) {
    throw new Error('Unexpected cargo-packager PKGBUILD layout')
  }
  recipe = recipe.replace('options=(!lto)', 'options=(!lto !strip !debug)')
    .replace('pkgrel=1\n', "pkgrel=1\nlicense=('MIT')\n")
    // makepkg also places a source-archive symlink in srcdir. Copying its
    // wildcard would install a dangling reference to our temporary stage.
    .replace(copyPayload, '\tcp -a -- "${srcdir}/usr" "${pkgdir}/"')
  writeFileSync(recipePath, recipe)
  writeFileSync(path.join(directory, 'PKGBUILD'), recipe)
  const archive = `${binaryName}_${version}_x86_64.tar.gz`
  copyFileSync(path.join(outDir, archive), path.join(directory, archive))
  run('makepkg', ['--noconfirm', '--force'], {
    cwd: directory, env: { PKGDEST: outDir, ZSTD_CLEVEL: process.env.ZSTD_CLEVEL ?? '15' }
  })
}

async function main() {
  if (Number(process.versions.node.split('.')[0]) < 20) throw new Error('Node.js 20 or later is required')
  const target = hostTarget()
  const formats = packageFormats(process.argv.slice(2), [releaseTargets[target].available[0]])
  const nativeFormats = formats.filter(format => format !== 'portable' && format !== 'appimage')
  if (process.platform === 'linux') {
    const tools = ['objcopy', 'strip']
    if (formats.includes('deb')) tools.push('ar', 'gzip')
    if (formats.includes('deb') || formats.includes('portable')) tools.push('xz')
    for (const tool of tools) run(tool, ['--version'], { capture: true })
  }
  if (nativeFormats.length) {
    const actual = run('cargo', ['packager', '--version'], { capture: true }).trim().split(/\s+/).at(-1)
    if (actual !== packagerVersion) throw new Error(`Install cargo-packager ${packagerVersion}: cargo install cargo-packager --version ${packagerVersion} --locked`)
  }
  if (formats.includes('appimage')) {
    for (const tool of ['linuxdeploy', 'appimagetool']) run(tool, ['--version'], { env: appImageToolEnv })
  }
  const metadata = JSON.parse(run('cargo', ['metadata', '--manifest-path', path.join(root, 'apps', 'desktop', 'Cargo.toml'),
    '--locked', '--filter-platform', target, '--format-version', '1'], { capture: true }))
  const desktop = metadata.packages.find(pkg => pkg.name === 'super-paste-desktop' && metadata.workspace_members.includes(pkg.id))
  if (!desktop) throw new Error('Independent desktop package is missing from workspace metadata')
  const version = desktop.version
  if (version !== workspaceVersion) throw new Error('Desktop version does not match workspace.package.version')
  const builtExecutable = path.join(root, 'target', target, 'release', `${binaryName}${process.platform === 'win32' ? '.exe' : ''}`)
  const dist = path.join(root, 'dist')
  const outDir = path.join(dist, target, version)
  mkdirSync(outDir, { recursive: true })
  const stage = mkdtempSync(path.join(dist, '.package-'))
  try {
    const resources = path.join(stage, 'resources')
    stageDependencyNotices({ metadata, desktop, root, target, directory: resources })
    run(process.execPath, [path.join(root, 'scripts', 'prepare-ort.mjs'), '--target', target])
    run('cargo', ['build', '--manifest-path', path.join(root, 'Cargo.toml'), '--locked', '--release',
      '--package', desktop.name, '--bin', binaryName, '--target', target, '--target-dir', path.join(root, 'target')], {
      env: process.platform === 'darwin' ? { MACOSX_DEPLOYMENT_TARGET: packaging.macos.minimumSystemVersion } : {}
    })
    if (!existsSync(builtExecutable) || !statSync(builtExecutable).isFile()) throw new Error(`Release executable is missing: ${builtExecutable}`)
    const linuxBinary = process.platform === 'linux' ? stageLinuxExecutable(builtExecutable, stage, run) : null
    const payload = linuxBinary?.executable ?? builtExecutable
    stageResources(target, resources)
    const files = regularFiles(resources)
    writeFileSync(path.join(resources, 'package-resources.json'), JSON.stringify({
      applicationId: 'com.v2dawn.superpaste.rs', version, target,
      files: files.map(file => ({ file, sha256: sha256(path.join(resources, file)) }))
    }, null, 2) + '\n')
    if (nativeFormats.length) {
      const config = JSON.parse(readFileSync(path.join(root, 'packaging', 'packager.json'), 'utf8'))
      // Config paths must survive a packager changing its working directory.
      config.licenseFile = path.join(root, config.licenseFile)
      config.icons = config.icons.map(file => path.join(root, file))
      config.deb.desktopTemplate = path.join(root, config.deb.desktopTemplate)
      config.macos.infoPlistPath = path.join(root, config.macos.infoPlistPath)
      config.nsis.installerIcon = path.join(root, config.nsis.installerIcon)
      Object.assign(config, {
        version, targetTriple: target, formats: nativeFormats, outDir,
        binariesDir: path.dirname(payload), binaries: [{ path: binaryName, main: true }],
        resources: [...files, 'package-resources.json'].map(file => ({
          src: path.join(resources, file), target: process.platform === 'win32' ? `resources/${file}` : file
        }))
      })
      const identity = process.env.SUPER_PASTE_MAC_SIGNING_IDENTITY
      if (process.platform === 'darwin' && identity) config.macos.signingIdentity = identity
      // Match the installed desktop ID to GPUI's Wayland app_id.
      if (process.platform === 'linux') {
        const entry = stageLinuxDesktop(root, stage, config)
        config.linux = { ...config.linux, generateDesktopEntry: false }
        for (const format of ['deb', 'pacman']) {
          config[format].files = { ...config[format].files,
            [entry.file]: `usr/share/applications/${entry.name}` }
        }
      }
      const configFile = path.join(stage, 'packager.json')
      writeFileSync(configFile, JSON.stringify(config, null, 2) + '\n')
      run('cargo', ['packager', '--config', configFile])
      if (formats.includes('deb')) compressDeb(path.join(outDir, `${binaryName}_${version}_amd64.deb`), stage, run)
      if (nativeFormats.includes('pacman')) pacman(stage, outDir, version)
    }
    if (formats.includes('appimage')) await packageAppImage({ root, stage, payload, resources, outDir, version, run })
    if (formats.includes('portable')) portable(stage, payload, outDir, target, version)
    if (linuxBinary) {
      const symbolDirectory = path.join(outDir, 'symbols')
      mkdirSync(symbolDirectory, { recursive: true })
      renameSync(linuxBinary.symbols, path.join(symbolDirectory, path.basename(linuxBinary.symbols)))
    }
    console.log(`Packages written to ${outDir}`)
  } finally {
    rmSync(stage, { recursive: true, force: true })
  }
}

try { await main() } catch (error) { console.error(error.message); process.exitCode = 1 }
