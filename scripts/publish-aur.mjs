/* eslint-disable @typescript-eslint/explicit-function-return-type -- Node.js 直接执行的 JavaScript 脚本。 */
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  copyFileSync,
  createReadStream,
  createWriteStream,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, join } from 'node:path'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { prepareAur, writeSrcinfo } from './prepare-aur.mjs'

const variant = process.argv[2]
if (!['git', 'bin'].includes(variant)) throw new Error('Usage: publish-aur.mjs git|bin')
const repository = process.env.GITHUB_REPOSITORY
if (!repository || !process.env.GITHUB_TOKEN)
  throw new Error('GITHUB_REPOSITORY and GITHUB_TOKEN are required')
async function api(path = '') {
  const response = await fetch(`https://api.github.com/repos/${repository}${path}`, {
    headers: {
      Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      Accept: 'application/vnd.github+json'
    }
  })
  if (!response.ok) throw new Error(`GitHub ${path}: ${response.status} ${await response.text()}`)
  return response.json()
}
const metadata = await api()
if (metadata.private)
  throw new Error('AUR requires a public source repository and public release assets')
const upstream = 'https://github.com/' + repository
const work = mkdtempSync(join(tmpdir(), 'super-paste-aur-'))
const value = (text, key) => new RegExp(`^\\s*${key} = (\\S+)`, 'm').exec(text)?.[1]
try {
  let artifact
  let version
  if (variant === 'bin') {
    const tag = process.env.AUR_RELEASE_TAG?.trim()
    const release = await api(
      tag ? `/releases/tags/${encodeURIComponent(tag)}` : '/releases/latest'
    )
    if (release.draft || !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(release.tag_name))
      throw new Error('AUR bin requires a published v<version> release')
    version = release.tag_name.slice(1)
    const filename = `super_paste_app_${version}_amd64.deb`
    const asset = release.assets.find((item) => item.name === filename)
    if (!asset) throw new Error(`Release ${release.tag_name} has no ${filename}`)
    artifact = join(work, filename)
    // 不带凭据下载，确保 AUR 用户也能访问同一个发行文件。
    const response = await fetch(asset.browser_download_url)
    if (!response.ok || !response.body)
      throw new Error(`Public release download failed: ${response.status}`)
    await pipeline(Readable.fromWeb(response.body), createWriteStream(artifact))
    if (asset.digest) {
      const hash = createHash('sha256')
      for await (const chunk of createReadStream(artifact)) hash.update(chunk)
      if (asset.digest !== `sha256:${hash.digest('hex')}`)
        throw new Error('Release asset checksum mismatch')
    }
  }
  const [prepared] = await prepareAur({ variant, artifact, version })
  const recipe = join(prepared.directory, 'PKGBUILD')
  writeFileSync(recipe, readFileSync(recipe, 'utf8').replace(/^url=.*$/m, `url='${upstream}'`))
  writeSrcinfo(prepared.directory)
  const destination = join(work, 'aur')
  execFileSync(
    'git',
    ['clone', `ssh://aur@aur.archlinux.org/${basename(prepared.directory)}.git`, destination],
    { stdio: 'inherit' }
  )
  const git = (args) => execFileSync('git', args, { cwd: destination, stdio: 'inherit' })
  const existing = existsSync(join(destination, '.SRCINFO'))
    ? readFileSync(join(destination, '.SRCINFO'), 'utf8')
    : ''
  const previousVersion = value(existing, 'pkgver')
  const nextVersion = value(readFileSync(join(prepared.directory, '.SRCINFO'), 'utf8'), 'pkgver')
  const sameVersion = nextVersion === previousVersion
  if (previousVersion) {
    const comparison = execFileSync('vercmp', [nextVersion, previousVersion], {
      encoding: 'utf8'
    }).trim()
    if (Number(comparison) < 0)
      throw new Error(`Refusing AUR downgrade: ${previousVersion} -> ${nextVersion}`)
  }
  const previousRel = value(existing, 'pkgrel')
  if (previousVersion && !/^[1-9]\d*$/.test(previousRel || ''))
    throw new Error('Invalid existing AUR pkgrel')
  const setPkgrel = (pkgrel) => {
    writeFileSync(recipe, readFileSync(recipe, 'utf8').replace(/^pkgrel=.*$/m, `pkgrel=${pkgrel}`))
    writeSrcinfo(prepared.directory)
    for (const file of prepared.files)
      copyFileSync(join(prepared.directory, file), join(destination, file))
    git(['add', '--', ...prepared.files])
  }
  setPkgrel(sameVersion ? previousRel : 1)
  const changes = spawnSync('git', ['diff', '--cached', '--quiet'], { cwd: destination })
  if (changes.error || ![0, 1].includes(changes.status))
    throw changes.error || new Error('Cannot compare AUR changes')
  if (changes.status === 0) console.log('AUR package is already current')
  else {
    if (sameVersion) setPkgrel(Number(previousRel) + 1)
    git(['config', 'user.name', process.env.AUR_GIT_NAME || 'Teddy Zhu'])
    git(['config', 'user.email', process.env.AUR_GIT_EMAIL || 'teddyzhu15@gmail.com'])
    git(['commit', '-m', `Update ${basename(prepared.directory)} to ${nextVersion}`])
    git(['push', 'origin', 'HEAD:master'])
    console.log(`Published ${basename(prepared.directory)} ${nextVersion}`)
  }
} finally {
  rmSync(work, { recursive: true, force: true })
}
