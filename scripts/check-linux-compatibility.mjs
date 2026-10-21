/* eslint-disable @typescript-eslint/explicit-function-return-type -- Node.js 直接执行的 JavaScript 脚本。 */
import { execFileSync } from 'node:child_process'
import { closeSync, openSync, readSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

const root = resolve(process.argv[2] || 'dist/linux-unpacked')
const limits = {
  GLIBC: process.env.SUPER_PASTE_GLIBC_MAX || '2.35',
  GLIBCXX: process.env.SUPER_PASTE_GLIBCXX_MAX || '3.4.30'
}
for (const limit of Object.values(limits)) {
  if (!/^\d+(?:\.\d+)+$/.test(limit)) throw new Error(`Invalid compatibility version: ${limit}`)
}
const compare = (a, b) => {
  const left = a.split('.').map(Number)
  const right = b.split('.').map(Number)
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const difference = (left[i] || 0) - (right[i] || 0)
    if (difference) return difference
  }
  return 0
}
let count = 0
const violations = []
const highest = {}
function inspect(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const file = join(directory, entry.name)
    if (entry.isDirectory()) {
      inspect(file)
      continue
    }
    if (!entry.isFile()) continue
    const fd = openSync(file, 'r')
    const magic = Buffer.alloc(4)
    try {
      readSync(fd, magic, 0, 4, 0)
    } finally {
      closeSync(fd)
    }
    if (!magic.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) continue
    count++
    const versions = execFileSync('readelf', ['--version-info', '--wide', file], {
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024
    })
    const required = {}
    for (const [, library, version] of versions.matchAll(/\b(GLIBC|GLIBCXX)_(\d+(?:\.\d+)+)\b/g)) {
      if (!required[library] || compare(version, required[library]) > 0) required[library] = version
      if (!highest[library] || compare(version, highest[library]) > 0) highest[library] = version
    }
    for (const [library, version] of Object.entries(required)) {
      if (compare(version, limits[library]) > 0) {
        violations.push(`${file}: requires ${library}_${version}, maximum ${limits[library]}`)
      }
    }
  }
}
inspect(root)
if (!count) throw new Error(`No ELF files found in ${root}`)
if (violations.length)
  throw new Error(`${violations.join('\n')}\nRebuild with pnpm build:linux:portable.`)
console.log(
  `Checked ${count} ELF files: GLIBC_${highest.GLIBC}, GLIBCXX_${highest.GLIBCXX || 'none'}`
)
