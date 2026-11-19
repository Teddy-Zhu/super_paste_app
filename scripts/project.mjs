import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const manifest = readFileSync(path.join(projectRoot, 'Cargo.toml'), 'utf8')
const workspace = /^\[workspace\.package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m.exec(manifest)?.[1]
export const rustToolchain = /^channel\s*=\s*"([^"]+)"/m.exec(readFileSync(path.join(projectRoot, 'rust-toolchain.toml'), 'utf8'))?.[1]
if (!rustToolchain) throw new Error('Missing pinned Rust toolchain')
export const version = /^version\s*=\s*"([^"]+)"/m.exec(workspace ?? '')?.[1]
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? '')) throw new Error('Invalid workspace.package.version in Cargo.toml')
export const packaging = JSON.parse(readFileSync(path.join(projectRoot, 'packaging/packager.json'), 'utf8'))
export const buildTools = JSON.parse(readFileSync(path.join(projectRoot, 'packaging/build-tools.json'), 'utf8'))
export const packageName = packaging.name
export const binaryName = packageName
export const productName = packaging.productName
export const aurName = 'super-paste-bin'
export const packagesDirectory = path.join(projectRoot, 'release-assets')
export const releaseTargets = {
  'x86_64-unknown-linux-gnu': { host: 'linux-x64', formats: ['deb', 'appimage', 'portable'], available: ['deb', 'pacman', 'appimage', 'portable'], extensions: ['.deb', '.AppImage', '.tar.xz'] },
  'x86_64-pc-windows-msvc': { host: 'win32-x64', formats: ['nsis', 'portable'], available: ['nsis', 'portable'], extensions: ['.exe', '.zip'] },
  'x86_64-apple-darwin': { host: 'darwin-x64', formats: ['dmg', 'app'], available: ['dmg', 'app'], extensions: ['.dmg', '.app.tar.gz'] },
  'aarch64-apple-darwin': { host: 'darwin-arm64', formats: ['dmg', 'app'], available: ['dmg', 'app'], extensions: ['.dmg', '.app.tar.gz'] }
}
export function releaseAssetNames() {
  return [
    ...Object.entries(releaseTargets).flatMap(([target, spec]) => [
      ...spec.extensions.map(extension => `${packageName}_${version}_${target}${extension}`),
      `ort-runtime-manifest_${version}_${target}.json`
    ]),
    `${packageName}_${version}_x86_64.pkg.tar.zst`, `${aurName}.PKGBUILD`, `${aurName}.SRCINFO`
  ]
}
export function hostTarget() {
  const target = Object.keys(releaseTargets).find(target => releaseTargets[target].host === `${process.platform}-${process.arch}`)
  if (!target) throw new Error(`Unsupported native host: ${process.platform}-${process.arch}`)
  return target
}
export function packageFormats(args, defaults = releaseTargets[hostTarget()].formats) {
  if (!args.length) return defaults
  if (args.length !== 2 || args[0] !== '--formats') throw new Error('Usage: node scripts/build.mjs [--formats format,format]')
  const formats = [...new Set(args[1].split(','))]
  if (formats.some(format => !releaseTargets[hostTarget()].available.includes(format))) throw new Error(`Supported formats: ${releaseTargets[hostTarget()].available.join(', ')}`)
  return formats
}
export function verifyReleaseTag(tag) {
  if (tag !== `v${version}`) throw new Error(`Release tag ${tag ?? '<missing>'} does not match v${version}`)
  return version
}
export function archVersion(upstreamVersion) { return upstreamVersion.replace('-', 'pre').replaceAll('-', '_') }
export function githubRepository(override) {
  let repository = override ?? process.env.GITHUB_REPOSITORY
  if (!repository) {
    const remote = execFileSync('git', ['remote', 'get-url', 'origin'], { cwd: projectRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim()
    repository = /github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/.exec(remote)?.[1]
  }
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? '')) throw new Error('Provide --repository <owner/repo> or configure a GitHub origin')
  return repository
}
