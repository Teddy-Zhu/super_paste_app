import { githubApi } from './github.mjs'
import { githubRepository } from './project.mjs'

const pageSize = 100
const categoryByType = {
  feat: '新增功能',
  fix: '问题修复',
  perf: '性能优化',
  refactor: '代码重构',
  style: '界面与样式',
  docs: '文档更新',
  build: '构建与依赖',
  ci: '构建与依赖',
  chore: '构建与依赖',
  test: '测试更新'
}

async function releaseChanges(tag, commit, previousTag) {
  for (let page = 1; ; page += 1) {
    const releases = previousTag
      ? [{ tag_name: previousTag }]
      : await githubApi(`/releases?per_page=${pageSize}&page=${page}`)
    for (const release of releases) {
      if (release.draft || release.tag_name === tag || (!tag.includes('-') && release.prerelease))
        continue

      const comparison = await githubApi(
        `/compare/${encodeURIComponent(release.tag_name)}...${commit}?per_page=${pageSize}&page=1`
      )
      // A release from another branch must not define this build's change range.
      if (!['ahead', 'identical'].includes(comparison.status)) continue

      const commits = [...comparison.commits]
      for (let nextPage = 2; commits.length < comparison.total_commits; nextPage += 1) {
        const next = await githubApi(
          `/compare/${comparison.base_commit.sha}...${commit}?per_page=${pageSize}&page=${nextPage}`
        )
        if (next.commits.length === 0)
          throw new Error(`release notes comparison is incomplete: ${release.tag_name}...${commit}`)
        commits.push(...next.commits)
      }
      return { previousTag: release.tag_name, commits }
    }
    if (releases.length < pageSize) break
  }
  if (previousTag)
    throw new Error(`release notes base ${previousTag} is not an ancestor of ${commit}`)

  const commits = []
  for (let page = 1; ; page += 1) {
    const entries = await githubApi(`/commits?sha=${commit}&per_page=${pageSize}&page=${page}`)
    commits.push(...entries)
    if (entries.length < pageSize) break
  }
  return { previousTag: null, commits: commits.reverse() }
}

function escapeMarkdown(text) {
  return text.replace(/[\\`*_[\]<>]/g, '\\$&')
}

function changeCategory(subject) {
  const type = /^([a-z]+)(?:\([^)]+\))?!?:\s*/i.exec(subject)?.[1].toLowerCase()
  if (type && categoryByType[type]) return categoryByType[type]
  if (/^(新增|添加|支持)/.test(subject)) return '新增功能'
  if (/^(修复|解决)/.test(subject)) return '问题修复'
  if (/^(优化|提高)/.test(subject)) return '性能优化'
  if (/^重构/.test(subject)) return '代码重构'
  return '其他更新'
}

export async function generateReleaseNotes(tag, commit, startTag) {
  const { previousTag, commits } = await releaseChanges(tag, commit, startTag)
  const repository = `https://github.com/${githubRepository()}`
  const groups = new Map()
  for (const entry of commits) {
    const subject = entry.commit.message.split(/\r?\n/, 1)[0].trim()
    if (!subject) throw new Error(`release notes commit has no subject: ${entry.sha}`)
    const category = changeCategory(subject)
    if (!groups.has(category)) groups.set(category, [])
    groups
      .get(category)
      .push(
        `- ${escapeMarkdown(subject)} ([${entry.sha.slice(0, 7)}](${repository}/commit/${entry.sha}))`
      )
  }

  const sections = [
    `更新范围：${escapeMarkdown(previousTag ?? '首次发布')} → ${escapeMarkdown(tag)}，共 ${commits.length} 次提交。`
  ]
  for (const [category, changes] of groups)
    sections.push(`### ${category}\n\n${changes.join('\n')}`)
  if (commits.length === 0) sections.push('本次发布没有新增提交。')
  const history = previousTag
    ? `${repository}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(tag)}`
    : `${repository}/commits/${encodeURIComponent(tag)}`
  sections.push(`**完整更新记录**：${history}`, `<!-- super-paste-release:${commit} -->`)
  return `${sections.join('\n\n')}\n`
}
