import { createHash } from 'node:crypto'
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

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const projectRoot = path.resolve(scriptDir, '..')
const ortRoot = path.join(projectRoot, 'resources', 'ort')
const runtimeVersion = '1.28.0'
const ortApiVersion = 17
const ortCommit = 'da9b5e364c465de65c49d91e696cd6485270757f'
const managedBy = 'scripts/prepare-ort.mjs'
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

const releaseAssets = {
  'x86_64-pc-windows-msvc': {
    asset: 'onnxruntime-win-x64-1.28.0.zip',
    assetId: 489173573,
    sha256: 'abef733dacbe2f571547a7150b479b5cb9cc0df22f96c24983a42cadb1b4f8bc',
    library: 'onnxruntime.dll'
  },
  'x86_64-unknown-linux-gnu': {
    asset: 'onnxruntime-linux-x64-1.28.0.tgz',
    assetId: 489174677,
    sha256: 'a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407',
    library: 'libonnxruntime.so'
  },
  'aarch64-apple-darwin': {
    asset: 'onnxruntime-osx-arm64-1.28.0.tgz',
    assetId: 489174685,
    sha256: '1268b359718099bde2cedb55787f182a130067bc4f31e8c88478c445b850d3d8',
    library: 'libonnxruntime.dylib'
  },
  'x86_64-apple-darwin': {
    library: 'libonnxruntime.dylib',
    sourceBuild: true
  }
}

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

function listFiles(directory) {
  const files = []
  const pending = [directory]
  while (pending.length > 0) {
    const current = pending.pop()
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const item = path.join(current, entry.name)
      if (entry.isDirectory()) pending.push(item)
      else if (entry.isFile() || entry.isSymbolicLink()) {
        try {
          if (statSync(item).isFile()) files.push(item)
        } catch {
          // Broken links are not valid runtime files.
        }
      }
    }
  }
  return files
}

function sha256(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex')
}

function verifyExisting(target, directory) {
  const manifestPath = path.join(directory, 'runtime-manifest.json')
  if (!existsSync(manifestPath)) return false
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  if (
    manifest.managedBy !== managedBy ||
    manifest.runtimeVersion !== runtimeVersion ||
    manifest.ortApiVersion !== ortApiVersion ||
    manifest.target !== target
  ) {
    throw new Error(`existing ORT directory has an incompatible manifest: ${manifestPath}`)
  }
  for (const entry of [...manifest.libraries, ...manifest.notices]) {
    const file = path.join(directory, entry.file)
    if (!existsSync(file) || sha256(file) !== entry.sha256) {
      throw new Error(`staged ORT resource failed SHA256 verification: ${file}`)
    }
  }
  return true
}

function runtimeFileName(target) {
  return releaseAssets[target]?.library
}

function isMainLibrary(target, name) {
  if (target.includes('windows')) return name.toLowerCase() === 'onnxruntime.dll'
  if (target.includes('linux')) return /^libonnxruntime\.so(?:\..*)?$/.test(name)
  return /^libonnxruntime(?:\.\d+(?:\.\d+)*)?\.dylib$/.test(name)
}

function isRuntimeLibrary(target, name) {
  if (target.includes('windows')) return name.toLowerCase().endsWith('.dll')
  if (target.includes('linux')) return /\.so(?:\..*)?$/.test(name)
  return name.endsWith('.dylib')
}

function findFile(root, predicate, label) {
  const match = listFiles(root).find((file) => predicate(path.basename(file)))
  if (!match) throw new Error(`official ORT package does not contain ${label}`)
  return match
}

function findMainLibrary(root, target) {
  const matches = listFiles(root).filter((file) => isMainLibrary(target, path.basename(file)))
  matches.sort((left, right) => {
    const expected = runtimeFileName(target)
    return Number(path.basename(right) === expected) - Number(path.basename(left) === expected)
  })
  if (!matches[0]) throw new Error(`ONNX Runtime package has no primary library for ${target}`)
  return matches[0]
}

function copyRuntimeLibraries(target, mainLibrary, destination) {
  const sourceDirectory = path.dirname(mainLibrary)
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

  const primaryName = runtimeFileName(target)
  if (!primaryName || files.length === 0)
    throw new Error(`no runtime libraries found for ${target}`)

  const staged = new Map()
  for (const file of files) {
    const name = path.basename(file)
    if (isMainLibrary(target, name)) continue
    const output = path.join(destination, name)
    if (staged.has(name)) continue
    copyFile(file, output)
    staged.set(name, output)
  }

  const primaryPath = path.join(destination, primaryName)
  copyFile(mainLibrary, primaryPath)
  staged.set(primaryName, primaryPath)

  return [...staged.entries()].map(([file, fullPath]) => ({ file, sha256: sha256(fullPath) }))
}

function copyFile(source, destination) {
  copyFileSync(source, destination)
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
    run('tar', ['-xf', archive, '-C', destination])
  }
}

async function buildIntelMacRuntime(workDirectory) {
  const sourceRoot = path.join(workDirectory, 'onnxruntime')
  const repository = 'https://github.com/microsoft/onnxruntime.git'
  run('git', [
    'clone',
    '--depth',
    '1',
    '--branch',
    `v${runtimeVersion}`,
    '--recurse-submodules',
    '--shallow-submodules',
    repository,
    sourceRoot
  ])
  const revision = spawnSync('git', ['-C', sourceRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' })
  if (revision.error || revision.status !== 0)
    throw new Error('could not read the ONNX Runtime source revision')
  if (revision.stdout.trim() !== ortCommit) {
    throw new Error(`unexpected ONNX Runtime source commit: ${revision.stdout.trim()}`)
  }

  const buildEnvironment = { ...process.env, MACOSX_DEPLOYMENT_TARGET: '13.3' }
  run(
    path.join(sourceRoot, 'build.sh'),
    [
      '--config',
      'Release',
      '--build_shared_lib',
      '--parallel',
      '--compile_no_warning_as_error',
      '--skip_submodule_sync',
      '--cmake_extra_defines',
      'CMAKE_OSX_ARCHITECTURES=x86_64'
    ],
    { cwd: sourceRoot, env: buildEnvironment }
  )

  const outputDirectory = path.join(sourceRoot, 'build', 'MacOS', 'Release')
  const mainLibrary = findMainLibrary(outputDirectory, 'x86_64-apple-darwin')
  return {
    sourceRoot,
    mainLibrary,
    source: {
      kind: 'official-source-build',
      repository,
      tag: `v${runtimeVersion}`,
      commit: ortCommit,
      build: 'CPU shared library; CMAKE_OSX_ARCHITECTURES=x86_64; MACOSX_DEPLOYMENT_TARGET=13.3'
    }
  }
}

function copyRequiredNotice(sourceRoot, destination, name) {
  const directFile = path.join(sourceRoot, name)
  const file = existsSync(directFile)
    ? directFile
    : findFile(sourceRoot, (basename) => basename === name, name)
  copyFile(file, path.join(destination, name))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const target = options.target
  const release = releaseAssets[target]
  if (!release) throw new Error(`unsupported release target: ${target}`)
  assertHostTarget(target)

  const destination = path.join(ortRoot, target)
  if (existsSync(destination)) {
    const existingManifestPath = path.join(destination, 'runtime-manifest.json')
    if (!existsSync(existingManifestPath)) {
      throw new Error(`refusing to replace an unmanaged ORT directory: ${destination}`)
    }
    const existingManifest = JSON.parse(readFileSync(existingManifestPath, 'utf8'))
    if (existingManifest.managedBy !== managedBy) {
      throw new Error(
        `refusing to replace an ORT directory not created by this script: ${destination}`
      )
    }
  }
  if (options.check) {
    if (!verifyExisting(target, destination))
      throw new Error(`ORT resources are not prepared for ${target}`)
    console.log(`verified ONNX Runtime ${runtimeVersion} for ${target}`)
    return
  }

  if (!options.force && verifyExisting(target, destination)) {
    console.log(`ONNX Runtime ${runtimeVersion} is already prepared for ${target}`)
    return
  }

  mkdirSync(ortRoot, { recursive: true })
  const workDirectory = mkdtempSync(path.join(os.tmpdir(), 'super-paste-ort-'))
  const stageRoot = mkdtempSync(path.join(ortRoot, `.stage-${target}-`))
  const stageDirectory = path.join(stageRoot, target)
  mkdirSync(stageDirectory, { recursive: true })

  try {
    let mainLibrary
    let noticeRoot
    let source

    if (release.sourceBuild) {
      const built = await buildIntelMacRuntime(workDirectory)
      mainLibrary = built.mainLibrary
      noticeRoot = built.sourceRoot
      source = built.source
    } else {
      const archiveUrl = `https://github.com/microsoft/onnxruntime/releases/download/v${runtimeVersion}/${release.asset}`
      // The asset API bypasses github.com's anonymous release-download gateway.
      const downloadUrl = `https://api.github.com/repos/microsoft/onnxruntime/releases/assets/${release.assetId}`
      const headers = {
        Accept: 'application/octet-stream',
        'User-Agent': 'super-paste-ort-build'
      }
      const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN
      if (token) headers.Authorization = `Bearer ${token}`
      const archive = path.join(workDirectory, release.asset)
      await downloadAsset(downloadUrl, archive, headers)
      const archiveSha256 = sha256(archive)
      if (archiveSha256 !== release.sha256) {
        throw new Error(
          `official ONNX Runtime archive SHA256 mismatch: expected ${release.sha256}, got ${archiveSha256}`
        )
      }
      const extracted = path.join(workDirectory, 'extracted')
      extractArchive(archive, extracted)
      mainLibrary = findMainLibrary(extracted, target)
      noticeRoot = extracted
      source = { kind: 'official-release-asset', url: archiveUrl, downloadUrl, archiveSha256 }
    }

    const libraries = copyRuntimeLibraries(target, mainLibrary, stageDirectory)
    copyRequiredNotice(noticeRoot, stageDirectory, 'LICENSE')
    copyRequiredNotice(noticeRoot, stageDirectory, 'ThirdPartyNotices.txt')
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

    if (existsSync(destination)) {
      const previous = JSON.parse(
        readFileSync(path.join(destination, 'runtime-manifest.json'), 'utf8')
      )
      if (previous.managedBy !== managedBy) {
        throw new Error(
          `refusing to replace an ORT directory not created by this script: ${destination}`
        )
      }
      rmSync(destination, { recursive: true, force: true })
    }
    renameSync(stageDirectory, destination)
    console.log(`prepared ONNX Runtime ${runtimeVersion} CPU library: ${destination}`)
  } finally {
    rmSync(workDirectory, { recursive: true, force: true })
    rmSync(stageRoot, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(`ORT preparation failed: ${error.message}`)
  process.exitCode = 1
})
