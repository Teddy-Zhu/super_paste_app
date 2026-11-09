import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { githubApi, tagCommit } from './github.mjs'
import { projectRoot, verifyReleaseTag, version } from './project.mjs'

const tag =
  process.env.RELEASE_TAG?.trim() ||
  (process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : `v${version}`)
verifyReleaseTag(tag)
const commit = execFileSync('git', ['rev-parse', 'HEAD'], {
  cwd: projectRoot,
  encoding: 'utf8'
}).trim()
const [existingCommit, release] = await Promise.all([
  tagCommit(tag),
  githubApi(`/releases/tags/${encodeURIComponent(tag)}`, { allowMissing: true })
])
if (existingCommit && existingCommit !== commit) {
  throw new Error(`tag ${tag} points to ${existingCommit}, not selected commit ${commit}`)
}
if (process.env.GITHUB_REF_TYPE === 'tag' && !existingCommit) {
  throw new Error(`release tag ${tag} does not exist`)
}
if (release && !release.draft) throw new Error(`release ${tag} is already published`)
const outputs = { version, tag, commit }
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    Object.entries(outputs)
      .map(([key, value]) => `${key}=${value}\n`)
      .join('')
  )
}
console.log(JSON.stringify(outputs))
