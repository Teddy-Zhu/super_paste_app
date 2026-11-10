import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { githubApi, tagCommit } from './github.mjs'
import { githubRepository, productName, projectRoot, verifyReleaseTag } from './project.mjs'
import { generateReleaseNotes } from './release-notes.mjs'

const releaseRoot = path.resolve(process.argv[2] ?? 'release-assets')
const tag = process.env.RELEASE_TAG ?? process.env.GITHUB_REF_NAME
if (!tag || !process.env.GITHUB_TOKEN) throw new Error('RELEASE_TAG and GITHUB_TOKEN are required')

const version = verifyReleaseTag(tag)
const commit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: projectRoot,
  encoding: 'utf8'
}).trim()
if (process.env.RELEASE_COMMIT && process.env.RELEASE_COMMIT !== commit) {
  throw new Error('release checkout does not match the prepared commit')
}
const [existingCommit, release] = await Promise.all([
  tagCommit(tag),
  githubApi(`/releases/tags/${encodeURIComponent(tag)}`, { allowMissing: true })
])
if (existingCommit && existingCommit !== commit)
  throw new Error(`tag ${tag} does not match the built commit`)
if (release && !release.draft) throw new Error(`release ${tag} is already published`)
if (!existsSync(releaseRoot)) throw new Error(`release asset directory is missing: ${releaseRoot}`)

function walk(directory) {
  const files = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...walk(file))
    else if (entry.isFile() && entry.name !== 'SHA256SUMS') files.push(file)
  }
  return files.sort()
}

const assets = walk(releaseRoot)
if (assets.length === 0) throw new Error(`no release assets found in ${releaseRoot}`)
if (new Set(assets.map((file) => path.basename(file))).size !== assets.length)
  throw new Error('release asset filenames must be unique')
const sums = assets
  .map((file) => {
    const hash = createHash('sha256').update(readFileSync(file)).digest('hex')
    return `${hash}  ${path.basename(file)}`
  })
  .join('\n')
writeFileSync(path.join(releaseRoot, 'SHA256SUMS'), `${sums}\n`)

const releaseAssets = [...assets, path.join(releaseRoot, 'SHA256SUMS')]
const gh = (args) =>
  execFileSync('gh', [...args, '--repo', githubRepository()], {
    cwd: projectRoot,
    env: { ...process.env, GH_TOKEN: process.env.GITHUB_TOKEN },
    stdio: 'inherit'
  })
const existingNotes = release?.body?.trim()
const generatedLink = /^\*\*Full Changelog\*\*:\s+(https:\/\/github\.com\/\S+)$/.exec(
  existingNotes ?? ''
)?.[1]
const needsNotes = !existingNotes || Boolean(generatedLink)
const comparisonPrefix = `https://github.com/${githubRepository()}/compare/`
const comparison = generatedLink?.startsWith(comparisonPrefix)
  ? generatedLink.slice(comparisonPrefix.length).split('...').map(decodeURIComponent)
  : null
const startTag = comparison?.length === 2 && comparison[1] === tag ? comparison[0] : undefined
const temporary = mkdtempSync(path.join(os.tmpdir(), 'super-paste-release-'))
try {
  const notesArgs = []
  if (needsNotes) {
    const notesFile = path.join(temporary, 'release-notes.md')
    writeFileSync(notesFile, await generateReleaseNotes(tag, commit, startTag))
    notesArgs.push('--notes-file', notesFile)
  }

  // Keep incomplete uploads in a draft so a failed job can resume without replacing a public release.
  if (!release) {
    const args = [
      'release',
      'create',
      tag,
      '--draft',
      '--target',
      commit,
      '--title',
      `${productName} ${version}`,
      ...notesArgs
    ]
    if (version.includes('-')) args.push('--prerelease')
    gh(args)
  } else {
    gh(['release', 'edit', tag, '--target', commit, ...notesArgs])
  }
  gh(['release', 'upload', tag, ...releaseAssets, '--clobber'])
  gh(['release', 'edit', tag, '--draft=false', `--prerelease=${version.includes('-')}`])
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
