import { githubRepository } from './project.mjs'

export async function githubApi(resource, { allowMissing = false } = {}) {
  const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN
  if (!token) throw new Error('GITHUB_TOKEN or GH_TOKEN is required')
  const response = await fetch(`https://api.github.com/repos/${githubRepository()}${resource}`, {
    headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(30_000)
  })
  if (allowMissing && response.status === 404) return null
  if (!response.ok) throw new Error(`GitHub ${resource}: HTTP ${response.status}`)
  return response.json()
}

export async function tagCommit(tag) {
  const reference = await githubApi(`/git/ref/tags/${encodeURIComponent(tag)}`, {
    allowMissing: true
  })
  let object = reference?.object
  if (!object) return null
  for (let depth = 0; depth < 8; depth += 1) {
    if (object.type === 'commit') return object.sha
    if (object.type !== 'tag') break
    object = (await githubApi(`/git/tags/${object.sha}`)).object
  }
  throw new Error(`cannot resolve release tag ${tag} to a commit`)
}
