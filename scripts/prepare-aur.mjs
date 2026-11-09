import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync
} from 'node:fs'
import path from 'node:path'
import {
  githubRepository,
  packageName,
  packagesDirectory,
  productName,
  projectRoot,
  version,
  verifyReleaseTag
} from './project.mjs'

const templateFile = path.join(projectRoot, 'packaging/aur/super-paste-bin/PKGBUILD.in')
const template = readFileSync(templateFile, 'utf8')
const args = process.argv.slice(2)
const options = {}
for (let index = 0; index < args.length; index += 1) {
  const key = args[index]
  if (!['--version', '--deb', '--output', '--repository'].includes(key))
    throw new Error(`unknown argument: ${key}`)
  const value = args[++index]
  if (!value) throw new Error(`${key} requires a value`)
  options[key.slice(2)] = value
}

const repository = githubRepository(options.repository)
if (options.version && options.version !== version) {
  throw new Error(`requested version ${options.version} does not match project ${version}`)
}
if (process.env.GITHUB_REF_TYPE === 'tag') verifyReleaseTag(process.env.GITHUB_REF_NAME)

const deb = path.resolve(
  options.deb ??
    path.join(projectRoot, 'target/release/bundle/deb', `${productName}_${version}_amd64.deb`)
)
const output = path.resolve(options.output ?? packagesDirectory)
if (!existsSync(deb))
  throw new Error(`Debian package is missing: ${deb}. Run pnpm build:pacman first.`)
mkdirSync(output, { recursive: true })
const expectedDeb = `${packageName}_${version}_x86_64-unknown-linux-gnu.deb`

const controlFiles = execFileSync('bsdtar', ['-tf', deb], { encoding: 'utf8' }).split(/\r?\n/)
const controlArchive = controlFiles.find((file) => /^control\.tar\./.test(file))
if (!controlArchive) throw new Error('Debian package has no control archive')
const controlBytes = execFileSync('bsdtar', ['-xOf', deb, controlArchive])
const controlText = execFileSync('bsdtar', ['-xOf', '-', '--include', '*control'], {
  input: controlBytes,
  encoding: 'utf8'
})
if (!controlText.split(/\r?\n/).includes(`Version: ${version}`)) {
  throw new Error(`Debian package version does not match ${version}`)
}
if (!controlText.split(/\r?\n/).includes(`Package: ${packageName}`)) {
  throw new Error(`Debian package name does not match ${packageName}`)
}
if (!controlText.split(/\r?\n/).includes('Architecture: amd64')) {
  throw new Error('Pacman packaging requires an amd64 Debian package')
}
const canonicalDeb = path.join(output, expectedDeb)
if (deb !== canonicalDeb) copyFileSync(deb, canonicalDeb)

const sha256 = (value) => createHash('sha256').update(value).digest('hex')
const debHash = sha256(readFileSync(deb))
const license = path.join(projectRoot, 'LICENSE')
const licenseHash = sha256(readFileSync(license))
const packageVersion = version.replace('-', 'pre').replaceAll('-', '_')
const packageDirectory = path.join(output, 'aur', 'super-paste-bin')
mkdirSync(packageDirectory, { recursive: true })
const pkgbuild = template
  .replaceAll('@REPOSITORY@', repository)
  .replaceAll('@PKGVER@', packageVersion)
  .replaceAll('@VERSION@', version)
  .replaceAll('@DEB_SHA256@', debHash)
  .replaceAll('@LICENSE_SHA256@', licenseHash)
const pkgbuildFile = path.join(packageDirectory, 'PKGBUILD')
writeFileSync(pkgbuildFile, pkgbuild)
const srcinfo = execFileSync('makepkg', ['--printsrcinfo'], {
  cwd: packageDirectory,
  encoding: 'utf8'
})
writeFileSync(path.join(packageDirectory, '.SRCINFO'), srcinfo)

const sourceCache = mkdtempSync(path.join(output, '.aur-source-'))
try {
  copyFileSync(deb, path.join(sourceCache, expectedDeb))
  copyFileSync(license, path.join(sourceCache, 'LICENSE'))
  const environment = { ...process.env, SRCDEST: sourceCache }
  execFileSync('makepkg', ['--nodeps', '--noconfirm', '--force'], {
    cwd: packageDirectory,
    env: environment,
    stdio: 'inherit'
  })
  const packageFiles = readdirSync(packageDirectory).filter((name) => name.endsWith('.pkg.tar.zst'))
  if (packageFiles.length !== 1)
    throw new Error(`expected one Pacman package, found ${packageFiles.length}`)
  const pacmanName = `${packageName}_${version}_x86_64.pkg.tar.zst`
  copyFileSync(path.join(packageDirectory, packageFiles[0]), path.join(output, pacmanName))
  console.log(`generated AUR metadata and ${pacmanName}`)
} finally {
  for (const file of readdirSync(packageDirectory)) {
    if (file.endsWith('.pkg.tar.zst') || file === 'src' || file === 'pkg') {
      rmSync(path.join(packageDirectory, file), { recursive: true, force: true })
    }
  }
  rmSync(sourceCache, { recursive: true, force: true })
}
