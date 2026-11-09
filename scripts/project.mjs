import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const packageJson = JSON.parse(readFileSync(path.join(projectRoot, 'package.json'), 'utf8'))
const tauriConfig = JSON.parse(
  readFileSync(path.join(projectRoot, 'src-tauri/tauri.conf.json'), 'utf8')
)
const cargoManifest = readFileSync(path.join(projectRoot, 'src-tauri/Cargo.toml'), 'utf8')

export const version = packageJson.version
export const packageName = packageJson.name
export const productName = tauriConfig.productName
export const binaryName = /^name\s*=\s*"([^"]+)"/m.exec(cargoManifest)?.[1]
export const packagesDirectory = path.join(projectRoot, 'target', 'packages')

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`invalid application version: ${version}`)
}
if (tauriConfig.version !== '../package.json') {
  throw new Error('src-tauri/tauri.conf.json must reference ../package.json; run pnpm version:sync')
}
const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(cargoManifest)?.[1]
if (cargoVersion !== version) {
  throw new Error(
    `src-tauri/Cargo.toml version ${cargoVersion ?? '<missing>'} does not match package.json ${version}; run pnpm version:sync`
  )
}
if (!packageName || !productName || !binaryName)
  throw new Error('application identity is incomplete')

export function verifyReleaseTag(tag) {
  if (!tag) throw new Error('provide the v<version> release tag')
  if (tag !== `v${version}`) throw new Error(`release tag ${tag} does not match v${version}`)
  return version
}

export function archVersion(upstreamVersion) {
  return upstreamVersion.replace('-', 'pre').replaceAll('-', '_')
}

export function isCurrentInstaller(file) {
  const name = path.basename(file)
  // Build directories can retain installers from earlier product names or versions.
  const names = [productName, productName.replaceAll(' ', '_'), packageName, binaryName]
  return names.some((stem) =>
    [
      `${stem}_${version}_`,
      `${stem}_${version}.`,
      `${stem}-${version}-`,
      `${stem}-${version}.`
    ].some((prefix) => name.startsWith(prefix))
  )
}

export function githubRepository(override) {
  let repository = override ?? process.env.GITHUB_REPOSITORY
  if (!repository) {
    try {
      const remote = execFileSync('git', ['remote', 'get-url', 'origin'], {
        cwd: projectRoot,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore']
      }).trim()
      repository = /github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/.exec(remote)?.[1]
    } catch {
      throw new Error('provide --repository <owner/repo> or set GITHUB_REPOSITORY')
    }
  }
  if (!repository || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error('provide --repository <owner/repo> or configure a GitHub origin')
  }
  return repository
}
