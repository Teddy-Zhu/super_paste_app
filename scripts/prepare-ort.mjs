// Adapted from Super Paste, fc90106d5a25233fe45a6ec9facd1ad3ffc4823b; MIT.
import { randomUUID } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import {
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { setTimeout as delay } from 'node:timers/promises'
import { ORT_MANAGED_BY, readRuntimeManifest } from './ort-manifest.mjs'
import { sha256File as sha256 } from './file-hash.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const projectRoot = path.resolve(scriptDir, '..')
const ortRoot = path.join(projectRoot, 'resources', 'ort')
const runtimeSpec = JSON.parse(
  readFileSync(path.join(projectRoot, 'resources', 'ort-runtime.json'), 'utf8')
)
const { archives, ortApiVersion, releaseAssets } = runtimeSpec
const managedBy = ORT_MANAGED_BY
const downloadAttempts = 5
const downloadTimeoutMs = 5 * 60 * 1000
const maxRetryDelayMs = 60 * 1000
const transientNetworkCodes = new Set([
  'ECONNRESET',
  'ECONNREFUSED',
  'EHOSTUNREACH',
  'ENETUNREACH',
  'ENOTFOUND',
  'EAI_AGAIN',
  'ETIMEDOUT',
  'EPIPE',
  'ERR_STREAM_PREMATURE_CLOSE',
  'UND_ERR_CONNECT_TIMEOUT',
  'UND_ERR_HEADERS_TIMEOUT',
  'UND_ERR_BODY_TIMEOUT',
  'UND_ERR_SOCKET'
])

function parseArgs(argv) {
  const options = { target: detectHostTarget(), force: false, check: false }
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]
    if (value === '--target') {
      options.target = argv[++index]
      if (!options.target) throw new Error('--target requires a Rust target triple')
    } else if (value === '--force') {
      options.force = true
    } else if (value === '--check') {
      options.check = true
    } else {
      throw new Error(`unknown argument: ${value}`)
    }
  }
  return options
}

function detectHostTarget() {
  if (process.platform === 'win32' && process.arch === 'x64') return 'x86_64-pc-windows-msvc'
  if (process.platform === 'linux' && process.arch === 'x64') return 'x86_64-unknown-linux-gnu'
  if (process.platform === 'darwin' && process.arch === 'x64') return 'x86_64-apple-darwin'
  if (process.platform === 'darwin' && process.arch === 'arm64') return 'aarch64-apple-darwin'
  throw new Error(
    `unsupported host for the pinned CPU runtime: ${process.platform}-${process.arch}`
  )
}

function assertHostTarget(target) {
  if (target !== detectHostTarget()) {
    throw new Error(
      `ORT must be prepared on its native target host; host=${detectHostTarget()}, requested=${target}`
    )
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env ?? process.env,
    stdio: 'inherit',
    shell: false
  })
  if (result.error) throw new Error(`could not run ${command}: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`${command} exited with status ${result.status}`)
}

function verifyExisting(target, directory) {
  const manifestPath = path.join(directory, 'runtime-manifest.json')
  if (!existsSync(manifestPath)) return false
  const { entries } = readRuntimeManifest(manifestPath, runtimeSpec, target)
  for (const entry of entries) {
    const file = path.join(directory, entry.file)
    if (!existsSync(file) || sha256(file) !== entry.sha256.toLowerCase()) {
      throw new Error(`staged ORT resource failed SHA256 verification: ${file}`)
    }
  }
  return true
}

function isRuntimeLibrary(target, name) {
  if (target.includes('windows')) return name.toLowerCase().endsWith('.dll')
  if (target.includes('linux')) return /\.so(?:\..*)?$/.test(name)
  return name.endsWith('.dylib')
}

function copyRuntimeLibraries(target, sourceDirectory, destination) {
  const primaryName = releaseAssets[target].library
  if (!statSync(path.join(sourceDirectory, primaryName), { throwIfNoEntry: false })?.isFile()) {
    throw new Error(`ONNX Runtime package is missing ${primaryName} for ${target}`)
  }
  const files = readdirSync(sourceDirectory, { withFileTypes: true })
    .filter((entry) => {
      const item = path.join(sourceDirectory, entry.name)
      if (!(entry.isFile() || entry.isSymbolicLink())) return false
      try {
        return statSync(item).isFile() && isRuntimeLibrary(target, entry.name)
      } catch {
        return false
      }
    })
    .map((entry) => path.join(sourceDirectory, entry.name))

  return files.map(file => {
    const name = path.basename(file)
    const output = path.join(destination, name)
    copyFileSync(file, output)
    return { file: name, sha256: sha256(output) }
  })
}

function isTransientNetworkError(error) {
  return (
    ['AbortError', 'TimeoutError'].includes(error?.name) ||
    transientNetworkCodes.has(error?.code) ||
    (error?.cause && isTransientNetworkError(error.cause)) ||
    (error instanceof AggregateError && error.errors.some(isTransientNetworkError))
  )
}

function downloadRetryDelay(response, attempt) {
  const backoff = response?.status === 429 ? 60_000 : 2000 * 2 ** (attempt - 1)
  const retryAfter = response?.headers.get('retry-after')?.trim()
  if (!retryAfter) return backoff
  const requestedDelay = /^\d+$/.test(retryAfter)
    ? Number(retryAfter) * 1000
    : Date.parse(retryAfter) - Date.now()
  return Math.max(backoff, Number.isFinite(requestedDelay) ? requestedDelay : 0)
}

async function downloadAsset(url, destination, headers) {
  const partial = `${destination}.part`
  for (let attempt = 1; attempt <= downloadAttempts; attempt += 1) {
    let response
    try {
      console.log(`Downloading ONNX Runtime (attempt ${attempt}/${downloadAttempts}): ${url}`)
      const signal = AbortSignal.timeout(downloadTimeoutMs)
      response = await fetch(url, { signal, headers })
      if (!response.ok || !response.body) {
        await response.body?.cancel()
        const endpoint = new URL(response.url)
        const requestId =
          response.headers.get('x-github-request-id') || response.headers.get('x-ms-request-id')
        throw new Error(
          `HTTP ${response.status}${response.body ? '' : ' (empty body)'} at ${endpoint.origin}${endpoint.pathname}${requestId ? `; request-id=${requestId}` : ''}`
        )
      }
      await pipeline(Readable.fromWeb(response.body), createWriteStream(partial), { signal })
      renameSync(partial, destination)
      return
    } catch (error) {
      rmSync(partial, { force: true })
      const retryable =
        response && !response.ok
          ? response.status === 408 ||
            response.status === 429 ||
            (response.status >= 500 && response.status < 600)
          : isTransientNetworkError(error)
      const detail = error.cause?.message
        ? `${error.message}: ${error.cause.message}`
        : error.message
      const message = `download failed on attempt ${attempt}/${downloadAttempts}: ${url}; ${detail}`
      if (!retryable || attempt === downloadAttempts) throw new Error(message, { cause: error })
      const retryDelay = downloadRetryDelay(response, attempt)
      if (retryDelay > maxRetryDelayMs) {
        throw new Error(
          `${message}; Retry-After exceeds the ${maxRetryDelayMs / 1000}s automatic retry limit`,
          { cause: error }
        )
      }
      console.warn(
        `ORT download attempt ${attempt}/${downloadAttempts} failed (${detail}); retrying in ${Math.ceil(retryDelay / 1000)}s`
      )
      await delay(retryDelay)
    }
  }
}

function extractArchive(archive, destination) {
  mkdirSync(destination, { recursive: true })
  if (process.platform === 'win32') {
    // Native -Command arguments are parsed as code, so pass paths through the process environment.
    run(
      'powershell.exe',
      [
        '-NoLogo',
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        'Expand-Archive -LiteralPath $env:SUPER_PASTE_ARCHIVE_SOURCE -DestinationPath $env:SUPER_PASTE_ARCHIVE_DESTINATION -Force -ErrorAction Stop'
      ],
      {
        env: {
          ...process.env,
          SUPER_PASTE_ARCHIVE_SOURCE: archive,
          SUPER_PASTE_ARCHIVE_DESTINATION: destination
        }
      }
    )
  } else {
    run('unzip', ['-q', archive, '-d', destination])
  }
}

async function prepare(options) {
  const target = options.target
  const release = releaseAssets[target]
  if (!release) throw new Error(`unsupported release target: ${target}`)

  const { runtimeVersion } = release
  const archiveSpec = archives[runtimeVersion]
  const destination = path.join(ortRoot, target)
  let existingManifest
  if (existsSync(destination)) {
    const existingManifestPath = path.join(destination, 'runtime-manifest.json')
    if (!existsSync(existingManifestPath)) {
      throw new Error(`refusing to replace an unmanaged ORT directory: ${destination}`)
    }
    existingManifest = JSON.parse(readFileSync(existingManifestPath, 'utf8'))
    if (existingManifest.managedBy !== managedBy) {
      throw new Error(
        `refusing to replace an ORT directory not created by this script: ${destination}`
      )
    }
  }
  const sameArchive = existingManifest?.runtimeVersion === runtimeVersion &&
    existingManifest.source?.kind === 'official-nuget-package' &&
    existingManifest.source?.url === archiveSpec.url &&
    existingManifest.source.archiveSha256 === archiveSpec.sha256
  if (options.check) {
    if (!sameArchive || !verifyExisting(target, destination))
      throw new Error(`ORT resources are not prepared for ${target}`)
    console.log(`verified ONNX Runtime ${runtimeVersion} for ${target}`)
    return
  }

  if (!options.force && sameArchive && verifyExisting(target, destination)) {
    console.log(`ONNX Runtime ${runtimeVersion} is already prepared for ${target}`)
    return
  }

  mkdirSync(ortRoot, { recursive: true })
  const workDirectory = mkdtempSync(path.join(os.tmpdir(), 'super-paste-ort-'))
  const stageRoot = mkdtempSync(path.join(ortRoot, `.stage-${target}-`))
  const stageDirectory = path.join(stageRoot, target)
  mkdirSync(stageDirectory, { recursive: true })

  try {
    const archive = path.join(workDirectory, 'onnxruntime.zip')
    await downloadAsset(archiveSpec.url, archive, { 'User-Agent': 'super-paste-ort-build' })
    const archiveSha256 = sha256(archive)
    if (archiveSha256 !== archiveSpec.sha256) {
      throw new Error(
        `official ONNX Runtime archive SHA256 mismatch: expected ${archiveSpec.sha256}, got ${archiveSha256}`
      )
    }
    const extracted = path.join(workDirectory, 'extracted')
    extractArchive(archive, extracted)
    const source = { kind: 'official-nuget-package', url: archiveSpec.url, archiveSha256 }
    const libraries = copyRuntimeLibraries(target, path.join(extracted, release.nativeDirectory), stageDirectory)
    for (const notice of ['LICENSE', 'ThirdPartyNotices.txt']) {
      copyFileSync(path.join(extracted, notice), path.join(stageDirectory, notice))
    }
    const notices = ['LICENSE', 'ThirdPartyNotices.txt'].map((file) => ({
      file,
      sha256: sha256(path.join(stageDirectory, file))
    }))
    const manifest = {
      managedBy,
      runtime: 'ONNX Runtime CPU shared library',
      runtimeVersion,
      ortApiVersion,
      target,
      source,
      libraries,
      notices
    }
    writeFileSync(
      path.join(stageDirectory, 'runtime-manifest.json'),
      `${JSON.stringify(manifest, null, 2)}\n`
    )
    readRuntimeManifest(path.join(stageDirectory, 'runtime-manifest.json'), runtimeSpec, target)

    if (existsSync(destination)) {
      const previous = JSON.parse(
        readFileSync(path.join(destination, 'runtime-manifest.json'), 'utf8')
      )
      if (previous.managedBy !== managedBy) {
        throw new Error(
          `refusing to replace an ORT directory not created by this script: ${destination}`
        )
      }
      const backup = path.join(ortRoot, `.previous-${target}-${randomUUID()}`)
      renameSync(destination, backup)
      try {
        renameSync(stageDirectory, destination)
      } catch (installError) {
        try { renameSync(backup, destination) } catch (restoreError) {
          throw new AggregateError([installError, restoreError], `ORT replacement and restoration failed; previous resources remain at ${backup}`)
        }
        throw installError
      }
      rmSync(backup, { recursive: true, force: true })
    } else {
      renameSync(stageDirectory, destination)
    }
    console.log(`prepared ONNX Runtime ${runtimeVersion} CPU library: ${destination}`)
  } finally {
    rmSync(workDirectory, { recursive: true, force: true })
    rmSync(stageRoot, { recursive: true, force: true })
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  assertHostTarget(options.target)
  if (!releaseAssets[options.target]) throw new Error(`unsupported release target: ${options.target}`)
  mkdirSync(ortRoot, { recursive: true })
  const lock = path.join(ortRoot, `.lock-${options.target}`)
  try { mkdirSync(lock) } catch (error) {
    if (error.code === 'EEXIST') throw new Error(`ORT preparation is already locked: ${lock}`)
    throw error
  }
  try { await prepare(options) } finally { rmSync(lock, { recursive: true, force: true }) }
}

main().catch((error) => {
  console.error(`ORT preparation failed: ${error.message}`)
  process.exitCode = 1
})
