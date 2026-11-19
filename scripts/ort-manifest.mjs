import { readFileSync, statSync } from 'node:fs'

export const ORT_MANAGED_BY = 'super-paste-rs/scripts/prepare-ort.mjs'
const fileName = /^[a-z0-9][a-z0-9._-]*$/i
const sha256 = /^[a-f0-9]{64}$/i

// Preparation and packaging must accept exactly the same resource inventory.
// Digests describe the prepared bytes; each caller verifies its own file copy.
export function readRuntimeManifest(file, spec, target) {
  const metadata = statSync(file)
  if (!metadata.isFile() || metadata.size > 256 * 1024) {
    throw new Error(`Invalid or oversized ORT manifest: ${file}`)
  }
  const bytes = readFileSync(file)
  let manifest
  try { manifest = JSON.parse(bytes.toString('utf8')) } catch (cause) {
    throw new Error(`Invalid ORT manifest JSON: ${file}`, { cause })
  }
  const library = spec.releaseAssets?.[target]?.library
  if (typeof library !== 'string' || !fileName.test(library) ||
      !manifest || typeof manifest !== 'object' || Array.isArray(manifest) ||
      manifest.managedBy !== ORT_MANAGED_BY || manifest.target !== target ||
      manifest.runtimeVersion !== spec.runtimeVersion || manifest.ortApiVersion !== spec.ortApiVersion ||
      !Array.isArray(manifest.libraries) || !Array.isArray(manifest.notices)) {
    throw new Error(`ORT manifest does not match this application's runtime and target: ${file}`)
  }
  const entries = [...manifest.libraries, ...manifest.notices]
  const names = new Set()
  for (const entry of entries) {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry) ||
        typeof entry.file !== 'string' || !fileName.test(entry.file) ||
        typeof entry.sha256 !== 'string' || !sha256.test(entry.sha256) ||
        names.has(entry.file.toLowerCase())) {
      throw new Error(`ORT manifest contains an invalid or duplicate file entry: ${file}`)
    }
    names.add(entry.file.toLowerCase())
  }
  if (!manifest.libraries.some(entry => entry.file === library) ||
      !['LICENSE', 'ThirdPartyNotices.txt'].every(name => manifest.notices.some(entry => entry.file === name))) {
    throw new Error(`ORT manifest is missing its main library or license notices: ${file}`)
  }
  return { bytes, entries }
}
