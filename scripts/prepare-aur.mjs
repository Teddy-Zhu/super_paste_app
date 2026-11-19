import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { archVersion, aurName, githubRepository, packageName, packaging, packagesDirectory, projectRoot, version, verifyReleaseTag } from './project.mjs'
import { sha256File } from './file-hash.mjs'

const args = process.argv.slice(2), options = {}
for (let index = 0; index < args.length; index++) {
  const key = args[index]
  if (!['--version', '--deb', '--output', '--repository'].includes(key) || !args[index + 1]) throw new Error(`Invalid argument: ${key}`)
  options[key.slice(2)] = args[++index]
}
const repository = githubRepository(options.repository)
if (options.version && options.version !== version) throw new Error(`Requested version ${options.version} does not match ${version}`)
if (process.env.GITHUB_REF_TYPE === 'tag') verifyReleaseTag(process.env.GITHUB_REF_NAME)
const deb = path.resolve(options.deb ?? path.join(projectRoot, 'dist', 'x86_64-unknown-linux-gnu', version, `${packageName}_${version}_amd64.deb`))
const output = path.resolve(options.output ?? packagesDirectory)
if (!existsSync(deb)) throw new Error(`Debian package missing: ${deb}. Run node scripts/build.mjs --formats deb first.`)
const controlArchive = execFileSync('bsdtar', ['-tf', deb], { encoding: 'utf8' }).split(/\r?\n/).find(name => /^control\.tar\./.test(name))
if (!controlArchive) throw new Error('Debian package has no control archive')
const control = execFileSync('bsdtar', ['-xOf', '-', '--include', '*control'], { input: execFileSync('bsdtar', ['-xOf', deb, controlArchive]), encoding: 'utf8' }).split(/\r?\n/)
for (const field of [`Version: ${version}`, `Package: ${packageName}`, 'Architecture: amd64']) {
  if (!control.includes(field)) throw new Error(`Debian package must contain ${field}`)
}
mkdirSync(output, { recursive: true })
const debName = `${packageName}_${version}_x86_64-unknown-linux-gnu.deb`
if (deb !== path.join(output, debName)) copyFileSync(deb, path.join(output, debName))
const license = path.join(projectRoot, 'LICENSE')
const directory = path.join(output, 'aur', aurName)
mkdirSync(directory, { recursive: true })
const values = {
  REPOSITORY: repository, PKGVER: archVersion(version), VERSION: version,
  DEB_SHA256: sha256File(deb), LICENSE_SHA256: sha256File(license),
  DEPENDS: [...new Set([...packaging.pacman.depends, 'hicolor-icon-theme', 'desktop-file-utils'])].map(name => `'${name}'`).join(' ')
}
const template = readFileSync(path.join(projectRoot, 'packaging/aur', aurName, 'PKGBUILD.in'), 'utf8')
const recipe = template.replace(/@([A-Z0-9_]+)@/g, (_, key) => {
  if (!(key in values)) throw new Error(`Unknown AUR template placeholder: ${key}`)
  return values[key]
})
writeFileSync(path.join(directory, 'PKGBUILD'), recipe)
writeFileSync(path.join(directory, '.SRCINFO'), execFileSync('makepkg', ['--printsrcinfo'], { cwd: directory, encoding: 'utf8' }))
const cache = mkdtempSync(path.join(output, '.aur-source-'))
try {
  copyFileSync(deb, path.join(cache, debName)); copyFileSync(license, path.join(cache, 'LICENSE'))
  execFileSync('makepkg', ['--nodeps', '--noconfirm', '--force'], {
    cwd: directory, env: { ...process.env, SRCDEST: cache, ZSTD_CLEVEL: process.env.ZSTD_CLEVEL ?? '15' }, stdio: 'inherit'
  })
  const packages = readdirSync(directory).filter(name => name.endsWith('.pkg.tar.zst'))
  if (packages.length !== 1) throw new Error(`Expected one Arch package, found ${packages.length}`)
  copyFileSync(path.join(directory, packages[0]), path.join(output, `${packageName}_${version}_x86_64.pkg.tar.zst`))
  for (const [source, name] of [['PKGBUILD', `${aurName}.PKGBUILD`], ['.SRCINFO', `${aurName}.SRCINFO`]]) copyFileSync(path.join(directory, source), path.join(output, name))
  console.log(`Generated Arch package and ${aurName} metadata in ${output}`)
} finally {
  for (const file of readdirSync(directory)) if (file.endsWith('.pkg.tar.zst') || file === 'src' || file === 'pkg') rmSync(path.join(directory, file), { recursive: true, force: true })
  rmSync(cache, { recursive: true, force: true })
}
