import { spawnSync } from 'node:child_process'
import { chmodSync, createWriteStream, existsSync, mkdirSync, renameSync, rmSync, statSync } from 'node:fs'
import path from 'node:path'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { buildTools, projectRoot } from './project.mjs'
import { sha256File } from './file-hash.mjs'

export async function preparePackageTools(formats) {
  if (formats.some(format => !['portable', 'appimage'].includes(format))) {
    const installed = spawnSync('cargo', ['packager', '--version'], { encoding: 'utf8' })
    if (installed.error || installed.status !== 0 || installed.stdout.trim().split(/\s+/).at(-1) !== buildTools.cargoPackager) {
      throw new Error(`Install cargo-packager first: cargo install cargo-packager --version ${buildTools.cargoPackager} --locked`)
    }
  }
  if (!formats.includes('appimage')) return {}
  const directory = path.join(projectRoot, 'target', 'package-tools')
  mkdirSync(directory, { recursive: true })
  for (const tool of buildTools.linux) {
    const file = path.join(directory, tool.name)
    const valid = file => existsSync(file) && statSync(file).isFile() && statSync(file).size === tool.bytes && sha256File(file) === tool.sha256
    if (!valid(file)) {
      const temporary = `${file}.${process.pid}.part`
      try {
        const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN
        const response = await fetch(`https://api.github.com/repos/${tool.repository}/releases/assets/${tool.assetId}`, {
          headers: { Accept: 'application/octet-stream', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
          signal: AbortSignal.timeout(180_000)
        })
        if (!response.ok || !response.body) throw new Error(`Download ${tool.name}: HTTP ${response.status}`)
        let bytes = 0
        const limit = new Transform({ transform(chunk, _, done) {
          bytes += chunk.length
          done(bytes > tool.bytes ? new Error(`${tool.name} exceeds its pinned size`) : null, chunk)
        } })
        await pipeline(Readable.fromWeb(response.body), limit, createWriteStream(temporary, { flags: 'wx' }))
        if (!valid(temporary)) throw new Error(`${tool.name} checksum mismatch`)
        renameSync(temporary, file)
      } finally { rmSync(temporary, { force: true }) }
    }
    chmodSync(file, 0o755)
  }
  return { PATH: `${directory}${path.delimiter}${process.env.PATH ?? ''}` }
}
