import { chmodSync, copyFileSync, cpSync, lstatSync, mkdirSync, readFileSync, renameSync, symlinkSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { sha256File } from './file-hash.mjs'
import { prepareAppImageRuntime } from './prepare-appimage-runtime.mjs'
import { stageNativeNotices } from './package-native-notices.mjs'

export const appImageToolEnv = { APPIMAGE_EXTRACT_AND_RUN: '1', NO_STRIP: '1' }

export function stageLinuxExecutable(source, stage, run) {
  const directory = path.join(stage, 'bin')
  mkdirSync(directory)
  const executable = path.join(directory, path.basename(source))
  const symbols = path.join(stage, `${path.basename(source)}.debug`)
  run('objcopy', ['--only-keep-debug', source, symbols])
  run('strip', ['--strip-all', '-o', executable, source])
  run('objcopy', [`--add-gnu-debuglink=${symbols}`, executable])
  return { executable, symbols }
}

export function compressDeb(packageFile, stage, run) {
  const directory = path.join(stage, 'deb')
  mkdirSync(directory)
  const members = ['debian-binary', 'control.tar.gz', 'data.tar.gz']
  const actual = run('ar', ['t', packageFile], { capture: true }).trim().split(/\r?\n/)
  if (actual.join('\n') !== members.join('\n')) throw new Error('Unexpected Debian archive layout from cargo-packager')
  // Preserve the control archive and every byte of the uncompressed data tar.
  // This also preserves tar ownership and modes without extracting its files.
  run('ar', ['x', packageFile, ...members], { cwd: directory })
  run('gzip', ['-d', 'data.tar.gz'], { cwd: directory })
  run('xz', ['-6', '-T2', 'data.tar'], { cwd: directory })
  const repacked = path.join(directory, path.basename(packageFile))
  run('ar', ['crD', repacked, ...members.slice(0, 2), 'data.tar.xz'], { cwd: directory })
  renameSync(repacked, packageFile)
}

export function stageLinuxDesktop(root, directory, config) {
  const desktop = readFileSync(path.resolve(root, config.deb.desktopTemplate), 'utf8')
    .replaceAll('{{exec}}', config.name).replaceAll('{{icon}}', config.name)
    .replaceAll('{{name}}', config.productName).replaceAll('{{identifier}}', config.identifier)
  if (desktop.includes('{{')) throw new Error('Unsupported placeholder in Linux desktop entry')
  const name = `${config.identifier}.desktop`
  const file = path.join(directory, name)
  writeFileSync(file, desktop)
  return { file, name }
}

export async function packageAppImage({ root, stage, payload, resources, outDir, version, run }) {
  const runtime = await prepareAppImageRuntime(root)
  const config = JSON.parse(readFileSync(path.join(root, 'packaging', 'packager.json'), 'utf8'))
  const appDir = path.join(stage, `${config.name}.AppDir`)
  const bins = path.join(appDir, 'usr', 'bin')
  const resourceDir = path.join(appDir, 'usr', 'lib', config.name)
  const applications = path.join(appDir, 'usr', 'share', 'applications')
  const icons = path.join(appDir, 'usr', 'share', 'icons', 'hicolor', '256x256', 'apps')
  for (const dir of [bins, applications, icons]) mkdirSync(dir, { recursive: true })
  copyFileSync(payload, path.join(bins, config.name))
  cpSync(resources, resourceDir, { recursive: true })
  const entry = stageLinuxDesktop(root, applications, config)
  copyFileSync(path.join(root, 'resources', 'app-icon.png'), path.join(icons, `${config.name}.png`))
  symlinkSync(`usr/share/applications/${entry.name}`, path.join(appDir, entry.name))
  symlinkSync(`usr/share/icons/hicolor/256x256/apps/${config.name}.png`, path.join(appDir, `${config.name}.png`))
  symlinkSync(`${config.name}.png`, path.join(appDir, '.DirIcon'))
  const appRun = path.join(appDir, 'AppRun')
  copyFileSync(path.join(root, 'packaging', 'linux', 'AppRun'), appRun)
  chmodSync(appRun, 0o755)
  const runtimeNotices = path.join(appDir, 'usr', 'share', 'licenses', `${config.name}-appimage-runtime`)
  mkdirSync(runtimeNotices, { recursive: true })
  for (const file of ['appimage-runtime.LICENSE', 'appimage-runtime.json']) {
    copyFileSync(path.join(root, 'packaging', 'linux', file), path.join(runtimeNotices, file))
  }

  // Deploy dependencies before sealing the image. linuxdeploy patches RPATHs,
  // including nested ORT libraries, even when stripping has been disabled.
  run('linuxdeploy', ['--appdir', appDir], { env: appImageToolEnv })
  const manifest = JSON.parse(readFileSync(path.join(resources, 'package-resources.json'), 'utf8'))
  const protectedFiles = [...manifest.files, {
    file: 'package-resources.json', sha256: sha256File(path.join(resources, 'package-resources.json'))
  }]
  for (const { file, sha256 } of protectedFiles) {
    const destination = path.join(resourceDir, file)
    if (!lstatSync(destination).isFile()) throw new Error(`AppImage resource is not a regular file: ${file}`)
    if (sha256File(destination) !== sha256) {
      copyFileSync(path.join(resources, file), destination)
      if (sha256File(destination) !== sha256) throw new Error(`AppImage resource checksum mismatch: ${file}`)
    }
  }
  stageNativeNotices(appDir, config.name, run)
  // The main executable retains linuxdeploy's RPATH for bundled dependencies.
  // The original ORT bytes are loaded by absolute path from resourceDir.
  run('appimagetool', [
    '--runtime-file', runtime, '--comp', 'zstd',
    '--mksquashfs-opt=-b', '--mksquashfs-opt=512K',
    '--mksquashfs-opt=-Xcompression-level', '--mksquashfs-opt=19',
    appDir, path.join(outDir, `${config.name}_${version}_x86_64.AppImage`)
  ], {
    env: { ...appImageToolEnv, ARCH: 'x86_64' }
  })
}
