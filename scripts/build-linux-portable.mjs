import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readdirSync, renameSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const dist = join(root, 'dist')
mkdirSync(dist, { recursive: true })
const staging = mkdtempSync(join(dist, '.portable-'))
try {
  execFileSync(
    'docker',
    [
      'build',
      '--platform',
      'linux/amd64',
      '--file',
      'packaging/linux/Dockerfile',
      '--output',
      `type=local,dest=${staging}`,
      '.'
    ],
    { cwd: root, stdio: 'inherit' }
  )
  // 构建成功后替换同名产物，保留其他平台、架构和版本的文件。
  for (const file of readdirSync(staging)) {
    rmSync(join(dist, file), { recursive: true, force: true })
    renameSync(join(staging, file), join(dist, file))
  }
} finally {
  rmSync(staging, { recursive: true, force: true })
}
