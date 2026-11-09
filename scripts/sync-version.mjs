import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

// Do not import project.mjs here: this script repairs the manifests that it validates.
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const read = (file) => readFileSync(path.join(projectRoot, file), 'utf8')
const version = JSON.parse(read('package.json')).version
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version))
  throw new Error(`invalid application version: ${version}`)

const replaceVersion = (text, file) => {
  const field = /^version[ \t]*=[ \t]*"[^"]+"/m
  if (!field.test(text)) throw new Error(`${file} must contain an explicit package version`)
  return text.replace(field, `version = "${version}"`)
}

const config = JSON.parse(read('src-tauri/tauri.conf.json'))
config.version = '../package.json'
const cargo = read('src-tauri/Cargo.toml').split(/(?=^\[)/m)
const packageIndex = cargo.findIndex((section) => /^\[package\][ \t]*\r?\n/.test(section))
if (packageIndex < 0) throw new Error('src-tauri/Cargo.toml has no [package] section')
const packageName = /^name[ \t]*=[ \t]*"([^"]+)"/m.exec(cargo[packageIndex])?.[1]
if (!packageName) throw new Error('src-tauri/Cargo.toml has no package name')
cargo[packageIndex] = replaceVersion(cargo[packageIndex], 'src-tauri/Cargo.toml')

const lock = read('Cargo.lock').split(/(?=^\[\[package\]\])/m)
const lockIndexes = lock.flatMap((section, index) => {
  const name = /^name[ \t]*=[ \t]*"([^"]+)"/m.exec(section)?.[1]
  return name === packageName && !/^source[ \t]*=/m.test(section) ? [index] : []
})
if (lockIndexes.length !== 1)
  throw new Error(`Cargo.lock must contain one workspace package named ${packageName}`)
lock[lockIndexes[0]] = replaceVersion(lock[lockIndexes[0]], 'Cargo.lock')

// Validate every input before updating any manifest; dependency versions stay unchanged.
for (const [file, text] of [
  ['src-tauri/tauri.conf.json', `${JSON.stringify(config, null, 2)}\n`],
  ['src-tauri/Cargo.toml', cargo.join('')],
  ['Cargo.lock', lock.join('')]
]) {
  if (read(file) === text) continue
  writeFileSync(path.join(projectRoot, file), text)
  console.log(`Synchronized ${file} with package.json ${version}`)
}
