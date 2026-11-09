import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  copyFileSync,
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from 'node:fs'
import path from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { githubApi } from './github.mjs'
import { archVersion, githubRepository, packageName, projectRoot } from './project.mjs'

const requestedTag = process.env.AUR_RELEASE_TAG?.trim()
if (requestedTag && !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(requestedTag))
  throw new Error('AUR_RELEASE_TAG must be a v<version> tag')
const [repository, release] = await Promise.all([
  githubApi(''),
  githubApi(
    requestedTag ? `/releases/tags/${encodeURIComponent(requestedTag)}` : '/releases/latest'
  )
])
if (repository.private)
  throw new Error('AUR requires a public repository and public release assets')
if (release.draft || !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(release.tag_name))
  throw new Error('AUR requires a published v<version> release')
const upstreamVersion = release.tag_name.slice(1)
const aurName = `${packageName}-bin`
const debName = `${packageName}_${upstreamVersion}_x86_64-unknown-linux-gnu.deb`
const recipeName = `${aurName}.PKGBUILD`
const metadataName = `${aurName}.SRCINFO`
const upstream = `https://github.com/${githubRepository()}`
const temporaryRoot = path.join(projectRoot, 'target', 'aur-publish')
mkdirSync(temporaryRoot, { recursive: true })
const work = mkdtempSync(path.join(temporaryRoot, 'publish-'))

async function sha256(file) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest('hex')
}

async function download(url, destination) {
  // Public downloads must also work without the workflow's GitHub credentials.
  const response = await fetch(url, { signal: AbortSignal.timeout(120_000) })
  if (!response.ok || !response.body)
    throw new Error(`public download failed: HTTP ${response.status}: ${url}`)
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destination))
}

async function downloadAsset(name) {
  const asset = release.assets.find((item) => item.name === name)
  if (!asset) throw new Error(`release ${release.tag_name} has no ${name}`)
  const file = path.join(work, name)
  await download(asset.browser_download_url, file)
  if (asset.digest && asset.digest !== `sha256:${await sha256(file)}`)
    throw new Error(`GitHub asset checksum mismatch: ${name}`)
  return file
}

const value = (text, key) => new RegExp(`^\\s*${key} = (\\S+)`, 'm').exec(text)?.[1]
try {
  const manifest = await downloadAsset('SHA256SUMS')
  const hashes = new Map(
    readFileSync(manifest, 'utf8')
      .trim()
      .split(/\r?\n/)
      .map((line) => {
        const match = /^([a-f0-9]{64}) {2}(.+)$/.exec(line)
        if (!match) throw new Error('invalid release SHA256SUMS')
        return [match[2], match[1]]
      })
  )
  for (const name of [recipeName, metadataName, debName]) {
    const file = await downloadAsset(name)
    if (hashes.get(name) !== (await sha256(file)))
      throw new Error(`release SHA256SUMS mismatch: ${name}`)
  }
  const prepared = path.join(work, 'package')
  const sourceCache = path.join(work, 'sources')
  mkdirSync(prepared)
  mkdirSync(sourceCache)
  const recipe = path.join(prepared, 'PKGBUILD')
  copyFileSync(path.join(work, recipeName), recipe)
  copyFileSync(path.join(work, debName), path.join(sourceCache, debName))
  await download(
    `https://raw.githubusercontent.com/${githubRepository()}/${release.tag_name}/LICENSE`,
    path.join(sourceCache, 'LICENSE')
  )
  const writeSrcinfo = () => {
    const text = execFileSync('makepkg', ['--printsrcinfo'], { cwd: prepared, encoding: 'utf8' })
    writeFileSync(path.join(prepared, '.SRCINFO'), text)
    return text
  }
  const next = writeSrcinfo()
  const nextVersion = value(next, 'pkgver')
  const nextEpoch = value(next, 'epoch') ?? '0'
  const expectedSource = `${debName}::${upstream}/releases/download/${release.tag_name}/${debName}`
  const source = next
    .split(/\r?\n/)
    .map((line) => line.trim())
    .includes(`source = ${expectedSource}`)
  if (
    value(next, 'pkgbase') !== aurName ||
    value(next, 'pkgname') !== aurName ||
    nextVersion !== archVersion(upstreamVersion) ||
    !source
  )
    throw new Error('AUR metadata does not match the selected release')
  execFileSync('makepkg', ['--verifysource', '--noconfirm'], {
    cwd: prepared,
    env: { ...process.env, SRCDEST: sourceCache },
    stdio: 'inherit'
  })

  const destination = path.join(work, 'aur')
  execFileSync('git', ['clone', `ssh://aur@aur.archlinux.org/${aurName}.git`, destination], {
    stdio: 'inherit'
  })
  const git = (args) => execFileSync('git', args, { cwd: destination, stdio: 'inherit' })
  const previous = existsSync(path.join(destination, '.SRCINFO'))
    ? readFileSync(path.join(destination, '.SRCINFO'), 'utf8')
    : ''
  const previousVersion = value(previous, 'pkgver')
  const previousEpoch = value(previous, 'epoch') ?? '0'
  const previousRel = value(previous, 'pkgrel')
  if (previousVersion) {
    const comparison = execFileSync(
      'vercmp',
      [`${nextEpoch}:${nextVersion}`, `${previousEpoch}:${previousVersion}`],
      { encoding: 'utf8' }
    ).trim()
    if (Number(comparison) < 0)
      throw new Error(
        `refusing AUR downgrade: ${previousEpoch}:${previousVersion} -> ${nextEpoch}:${nextVersion}`
      )
    if (!/^[1-9]\d*$/.test(previousRel ?? '')) throw new Error('invalid existing AUR pkgrel')
  }
  const sameVersion = previousVersion === nextVersion && previousEpoch === nextEpoch
  const setPkgrel = (pkgrel) => {
    writeFileSync(recipe, readFileSync(recipe, 'utf8').replace(/^pkgrel=.*$/m, `pkgrel=${pkgrel}`))
    writeSrcinfo()
    for (const file of ['PKGBUILD', '.SRCINFO'])
      copyFileSync(path.join(prepared, file), path.join(destination, file))
    git(['add', '--', 'PKGBUILD', '.SRCINFO'])
  }
  setPkgrel(sameVersion ? previousRel : '1')
  const changes = spawnSync('git', ['diff', '--cached', '--quiet'], { cwd: destination })
  if (changes.error || ![0, 1].includes(changes.status))
    throw changes.error ?? new Error('cannot compare AUR changes')
  if (changes.status === 0) console.log(`${aurName} is already current; nothing to publish`)
  else {
    if (sameVersion) setPkgrel(String(BigInt(previousRel) + 1n))
    git(['config', 'user.name', process.env.AUR_GIT_NAME || 'github-actions[bot]'])
    git([
      'config',
      'user.email',
      process.env.AUR_GIT_EMAIL || '41898282+github-actions[bot]@users.noreply.github.com'
    ])
    git(['commit', '-m', `Update ${aurName} to ${nextVersion}`])
    git(['push', 'origin', 'HEAD:master'])
    console.log(`published ${aurName} ${nextVersion}`)
  }
} finally {
  rmSync(work, { recursive: true, force: true })
}
