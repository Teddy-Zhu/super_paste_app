/* eslint-disable @typescript-eslint/explicit-function-return-type -- Node.js 直接执行的 JavaScript 脚本。 */
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  appendFileSync,
  createReadStream,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync
} from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repository = process.env.GITHUB_REPOSITORY
const git = (args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()
const marker = (sha) => `<!-- super-paste-release:${sha} -->`
async function api(path, { method = 'GET', body, allowMissing = false } = {}) {
  if (!repository || !process.env.GITHUB_TOKEN)
    throw new Error('GITHUB_REPOSITORY and GITHUB_TOKEN are required')
  const response = await fetch(`https://api.github.com/repos/${repository}/${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
      Accept: 'application/vnd.github+json',
      'Content-Type': 'application/json'
    },
    ...(body ? { body: JSON.stringify(body) } : {})
  })
  if (response.status === 404 && allowMissing) return null
  if (!response.ok)
    throw new Error(`GitHub ${method} ${path}: ${response.status} ${await response.text()}`)
  return response.status === 204 ? null : response.json()
}
function tagCommit(tag) {
  const result = spawnSync('git', ['show-ref', '--verify', '--quiet', `refs/tags/${tag}`], {
    cwd: root
  })
  if (result.status === 1) return null
  if (result.error || result.status !== 0)
    throw result.error || new Error(`Cannot resolve tag ${tag}`)
  return git(['rev-parse', `refs/tags/${tag}^{commit}`])
}
const escape = (text) =>
  text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replace(/[\\`*_[\]]/g, '\\$&')
export function renderNotes({ commits, previousTag, tag, repository }) {
  const url = `https://github.com/${repository}`
  const groups = new Map()
  const categories = {
    feat: '新增功能',
    fix: '问题修复',
    perf: '性能优化',
    refactor: '代码重构',
    build: '构建与依赖',
    ci: '构建与依赖',
    chore: '构建与依赖',
    docs: '文档更新'
  }
  for (const { sha, subject } of commits) {
    const type = /^([a-z]+)(?:\([^)]*\))?!?:/i.exec(subject)?.[1].toLowerCase()
    const category = categories[type] || '其他更新'
    if (!groups.has(category)) groups.set(category, [])
    groups.get(category).push(`- ${escape(subject)} ([${sha.slice(0, 7)}](${url}/commit/${sha}))`)
  }
  const range = previousTag
    ? `${escape(previousTag)} → ${escape(tag)}`
    : `首次发布 → ${escape(tag)}`
  const sections = [...groups].map(([title, lines]) => `### ${title}\n\n${lines.join('\n')}`)
  if (!commits.length) sections.push('与上次发布相比没有新增提交。')
  const comparison = previousTag
    ? `\n\n[完整差异](${url}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(tag)})`
    : ''
  return `更新范围：${range}，共 ${commits.length} 次提交。\n\n${sections.join('\n\n')}${comparison}\n`
}
async function prepare() {
  const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version))
    throw new Error(`Invalid release version: ${version}`)
  const tag = `v${version}`
  const sha = git(['rev-parse', 'HEAD'])
  const existingTag = tagCommit(tag)
  if (existingTag && existingTag !== sha)
    throw new Error(`${tag} points to another commit; bump package.json version`)
  const existing = await api(`releases/tags/${encodeURIComponent(tag)}`, { allowMissing: true })
  if (existing && (!existing.draft || !existing.body?.includes(marker(sha))))
    throw new Error(`${tag} already exists; bump package.json version`)
  const releases = []
  for (let page = 1; ; page++) {
    const batch = await api(`releases?per_page=100&page=${page}`)
    releases.push(...batch)
    if (batch.length < 100) break
  }
  let previousTag
  let previousCommit
  for (const release of releases
    .filter((item) => !item.draft && item.tag_name !== tag)
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at))) {
    const commit = tagCommit(release.tag_name)
    if (!commit)
      throw new Error(`Release tag ${release.tag_name} is missing; fetch the complete tag history`)
    const ancestor = spawnSync('git', ['merge-base', '--is-ancestor', commit, sha], { cwd: root })
    if (ancestor.error || ![0, 1].includes(ancestor.status))
      throw ancestor.error || new Error('Cannot determine release ancestry')
    if (ancestor.status === 0) {
      previousTag = release.tag_name
      previousCommit = commit
      break
    }
  }
  const lines = git([
    'log',
    '--reverse',
    '--format=%H%x09%s',
    previousCommit ? `${previousCommit}..${sha}` : sha
  ])
  const commitLines = lines ? lines.split('\n') : []
  const commits = commitLines.map((line) => ({
    sha: line.slice(0, line.indexOf('\t')),
    subject: line.slice(line.indexOf('\t') + 1)
  }))
  const notes = renderNotes({ commits, previousTag, tag, repository }) + `\n${marker(sha)}\n`
  if (Buffer.byteLength(notes) > 120000)
    throw new Error('Release notes exceed the GitHub body limit')
  const targets = [
    { platform: 'linux', arch: 'x64', runner: 'ubuntu-22.04' },
    { platform: 'win', arch: 'x64', runner: 'windows-2022' },
    { platform: 'mac', arch: 'arm64', runner: 'macos-15' },
    { platform: 'mac', arch: 'x64', runner: 'macos-15-intel' }
  ]
  const selected = process.env.RELEASE_PLATFORMS || 'linux'
  if (!['all', 'linux', 'win', 'mac'].includes(selected))
    throw new Error(`Invalid release platform: ${selected}`)
  const matrix = {
    include: targets.filter((target) => selected === 'all' || target.platform === selected)
  }
  mkdirSync(join(root, 'release-info'), { recursive: true })
  writeFileSync(join(root, 'release-info/release-notes.md'), notes)
  writeFileSync(
    join(root, 'release-info/manifest.json'),
    JSON.stringify({ tag, sha, version, previousTag })
  )
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(process.env.GITHUB_OUTPUT, `tag=${tag}\nmatrix=${JSON.stringify(matrix)}\n`)
  console.log(`Prepared ${tag}: ${commits.length} commits since ${previousTag || 'initial commit'}`)
}
async function publish() {
  const { tag, sha } = JSON.parse(readFileSync(join(root, 'release-info/manifest.json'), 'utf8'))
  if (git(['rev-parse', 'HEAD']) !== sha) throw new Error('Release source commit changed')
  const body = readFileSync(join(root, 'release-info/release-notes.md'), 'utf8')
  if (!body.includes(marker(sha))) throw new Error('Release notes do not match source commit')
  const directory = join(root, 'release-assets')
  const files = readdirSync(directory)
    .filter((file) => /\.(deb|pacman|AppImage|exe|dmg|zip)$/.test(file))
    .sort()
  if (!files.length) throw new Error('No release installers found')
  const sums = []
  for (const name of files) {
    const hash = createHash('sha256')
    for await (const chunk of createReadStream(join(directory, name))) hash.update(chunk)
    sums.push(`${hash.digest('hex')}  ${name}`)
  }
  writeFileSync(join(directory, 'SHA256SUMS'), sums.join('\n') + '\n')
  files.push('SHA256SUMS')
  let release = await api(`releases/tags/${encodeURIComponent(tag)}`, { allowMissing: true })
  if (release && (!release.draft || !release.body?.includes(marker(sha))))
    throw new Error(`Refusing to overwrite existing release ${tag}`)
  const prerelease = process.env.RELEASE_PRERELEASE === 'true'
  if (!release)
    release = await api('releases', {
      method: 'POST',
      body: { tag_name: tag, target_commitish: sha, name: tag, body, draft: true, prerelease }
    })
  for (const name of files) {
    const previous = release.assets.find((asset) => asset.name === name)
    if (previous) await api(`releases/assets/${previous.id}`, { method: 'DELETE' })
    const url = new URL(release.upload_url.replace(/\{.*$/, ''))
    if (url.hostname !== 'uploads.github.com') throw new Error('Unexpected release upload host')
    url.searchParams.set('name', name)
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
        'Content-Type': 'application/octet-stream',
        'Content-Length': String(statSync(join(directory, name)).size)
      },
      body: createReadStream(join(directory, name)),
      duplex: 'half'
    })
    if (!response.ok) throw new Error(`Upload ${name}: ${response.status} ${await response.text()}`)
  }
  release = await api(`releases/${release.id}`, {
    method: 'PATCH',
    body: { draft: false, prerelease, body }
  })
  if (process.env.GITHUB_STEP_SUMMARY)
    appendFileSync(
      process.env.GITHUB_STEP_SUMMARY,
      `Published [${tag}](${release.html_url}) with ${files.length} assets.\n`
    )
  console.log(`Published ${release.html_url}`)
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv[2] === 'prepare') await prepare()
  else if (process.argv[2] === 'publish') await publish()
  else throw new Error('Usage: github-release.mjs prepare|publish')
}
