/* eslint-disable @typescript-eslint/explicit-function-return-type -- Node.js 直接执行的 JavaScript 脚本。 */
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { copyFileSync, createReadStream, readFileSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const project = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))

export function archVersion(version = project.version) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version))
    throw new Error(`Invalid package version: ${version}`)
  // 以 pre 连接预发布后缀，确保 Arch 将其排在正式版本之前。
  return version.replace('-', 'pre').replaceAll('-', '_')
}

export function writeSrcinfo(directory) {
  const srcinfo = execFileSync('makepkg', ['--printsrcinfo'], { cwd: directory, encoding: 'utf8' })
  writeFileSync(join(directory, '.SRCINFO'), srcinfo)
}

export async function prepareAur({ variant = 'all', artifact, version = project.version } = {}) {
  if (!['git', 'bin', 'all'].includes(variant)) throw new Error(`Invalid AUR variant: ${variant}`)
  const packageVersion = archVersion(version)
  const prepared = []
  if (variant !== 'git') {
    const filename = `super_paste_app_${version}_amd64.deb`
    const file = resolve(artifact || join(root, 'dist', filename))
    if (basename(file) !== filename) throw new Error(`Expected release artifact: ${filename}`)
    const controlArchive = execFileSync('bsdtar', ['-xOf', file, 'control.tar.xz'])
    const control = execFileSync('bsdtar', ['-xOf', '-', '--include', '*control'], {
      input: controlArchive,
      encoding: 'utf8'
    })
    if (!control.split('\n').includes(`Version: ${version}`))
      throw new Error(`Debian package version does not match ${version}`)
    const directory = join(root, 'packaging/aur/super-paste-bin')
    copyFileSync(
      join(root, 'packaging/arch/super-paste.desktop'),
      join(directory, 'super-paste.desktop')
    )
    const hashes = []
    for (const asset of [
      file,
      join(directory, 'super-paste'),
      join(directory, 'super-paste.desktop')
    ]) {
      const hash = createHash('sha256')
      for await (const chunk of createReadStream(asset)) hash.update(chunk)
      hashes.push(hash.digest('hex'))
    }
    const build = join(directory, 'PKGBUILD')
    writeFileSync(
      build,
      readFileSync(build, 'utf8')
        .replace(/^pkgver=.*$/m, `pkgver=${packageVersion}`)
        // Arch 的 pkgver 不允许连字符，上游文件名与 Release 标签保留原版本。
        .replace(/^_upstreamver=.*$/m, `_upstreamver=${version}`)
        .replace(
          /^sha256sums=\([\s\S]*?\)/m,
          `sha256sums=(${hashes.map((hash) => `'${hash}'`).join('\n            ')})`
        )
    )
    prepared.push({
      directory,
      files: ['PKGBUILD', '.SRCINFO', 'super-paste', 'super-paste.desktop']
    })
  }
  if (variant !== 'bin') {
    const directory = join(root, 'packaging/aur/super-paste-git')
    const count = execFileSync('git', ['rev-list', '--count', 'HEAD'], {
      cwd: root,
      encoding: 'utf8'
    }).trim()
    const commit = execFileSync('git', ['rev-parse', '--short=7', 'HEAD'], {
      cwd: root,
      encoding: 'utf8'
    }).trim()
    const build = join(directory, 'PKGBUILD')
    writeFileSync(
      build,
      readFileSync(build, 'utf8').replace(
        /^pkgver=.*$/m,
        `pkgver=${archVersion()}.r${count}.g${commit}`
      )
    )
    prepared.push({ directory, files: ['PKGBUILD', '.SRCINFO'] })
  }
  for (const { directory } of prepared) writeSrcinfo(directory)
  return prepared
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values, positionals } = parseArgs({
    options: { variant: { type: 'string', default: 'all' }, version: { type: 'string' } },
    allowPositionals: true
  })
  if (positionals.length > 1 || (values.version && values.variant !== 'bin')) {
    throw new Error(
      'Usage: prepare-aur.mjs [--variant git|bin|all] [--version VERSION (bin only)] [deb]'
    )
  }
  await prepareAur({ variant: values.variant, version: values.version, artifact: positionals[0] })
  console.log(
    `Prepared AUR ${values.variant} metadata. Publish the matching deb without modification.`
  )
}
