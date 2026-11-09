import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { productName, verifyReleaseTag } from './project.mjs'

const releaseRoot = path.resolve(process.argv[2] ?? 'release-assets')
const tag = process.env.GITHUB_REF_NAME
if (!tag || !process.env.GITHUB_TOKEN)
  throw new Error('GITHUB_REF_NAME and GITHUB_TOKEN are required')

const version = verifyReleaseTag(tag)
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
const sums = assets
  .map((file) => {
    const hash = createHash('sha256').update(readFileSync(file)).digest('hex')
    return `${hash}  ${path.basename(file)}`
  })
  .join('\n')
writeFileSync(path.join(releaseRoot, 'SHA256SUMS'), `${sums}\n`)

const releaseAssets = [...assets, path.join(releaseRoot, 'SHA256SUMS')]
const args = [
  'release',
  'create',
  tag,
  ...releaseAssets,
  '--title',
  `${productName} ${version}`,
  '--generate-notes'
]
if (version.includes('-')) args.push('--prerelease')

execFileSync('gh', args, {
  cwd: releaseRoot,
  env: { ...process.env, GH_TOKEN: process.env.GITHUB_TOKEN },
  stdio: 'inherit'
})
