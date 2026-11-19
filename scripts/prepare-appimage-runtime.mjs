import { createWriteStream, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs'
import path from 'node:path'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { sha256File } from './file-hash.mjs'

export async function prepareAppImageRuntime(root) {
  const spec = JSON.parse(readFileSync(path.join(root, 'packaging', 'linux', 'appimage-runtime.json'), 'utf8'))
  const directory = path.join(root, 'target', 'package-tools')
  const file = path.join(directory, 'runtime-x86_64')
  const verify = candidate => {
    const info = lstatSync(candidate)
    if (!info.isFile() || info.size !== spec.bytes || sha256File(candidate) !== spec.sha256) {
      throw new Error(`Pinned AppImage runtime failed checksum verification: ${candidate}`)
    }
  }
  if (existsSync(file)) { verify(file); return file }
  mkdirSync(directory, { recursive: true })
  const temporary = mkdtempSync(path.join(directory, '.runtime-'))
  try {
    const response = await fetch(spec.url, { signal: AbortSignal.timeout(120_000) })
    if (!response.ok || !response.body) throw new Error(`AppImage runtime download failed: HTTP ${response.status}`)
    let bytes = 0
    const limit = new Transform({
      transform(chunk, encoding, done) {
        bytes += chunk.length
        done(bytes > spec.bytes ? new Error('AppImage runtime exceeds its pinned size') : null, chunk)
      }
    })
    const staged = path.join(temporary, 'runtime-x86_64')
    await pipeline(Readable.fromWeb(response.body), limit, createWriteStream(staged, { flags: 'wx' }))
    verify(staged)
    renameSync(staged, file)
    return file
  } finally {
    rmSync(temporary, { recursive: true, force: true })
  }
}
