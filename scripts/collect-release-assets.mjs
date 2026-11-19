import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readdirSync } from 'node:fs'
import path from 'node:path'
import { packageName, productName, projectRoot, version, releaseTargets } from './project.mjs'

const [target, outputArgument = 'release-assets'] = process.argv.slice(2)
const spec = releaseTargets[target]
if (!spec) throw new Error('Usage: collect-release-assets.mjs <supported-target> [output-directory]')
const directory = path.join(projectRoot, 'dist', target, version)
const output = path.resolve(outputArgument)
mkdirSync(output, { recursive: true })
const files = readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isFile()).map(entry => entry.name)
for (const extension of spec.extensions) {
  // Match this version exactly; cargo-packager uses different separators per format.
  const candidates = files.filter(name => [packageName, productName, productName.replaceAll(' ', '_')].some(stem =>
    name.startsWith(`${stem}_${version}_`) || name.startsWith(`${stem}-${version}-`) || name.startsWith(`${stem}_${version}.`)) && name.endsWith(extension))
  const name = `${packageName}_${version}_${target}${extension}`
  const destination = path.join(output, name)
  if (extension === '.app.tar.gz') {
    const apps = readdirSync(directory, { withFileTypes: true }).filter(entry => entry.isDirectory() && entry.name === `${productName}.app`)
    if (apps.length !== 1) throw new Error(`Expected one macOS app bundle in ${directory}`)
    execFileSync('tar', ['-czf', destination, '-C', directory, apps[0].name], { stdio: 'inherit' })
  } else {
    if (candidates.length !== 1) throw new Error(`Expected one ${extension} for ${target}, found ${candidates.length}`)
    if (path.resolve(directory, candidates[0]) !== destination) copyFileSync(path.join(directory, candidates[0]), destination)
  }
  console.log(`Collected ${name}`)
}
const manifest = path.join(projectRoot, 'resources', 'ort', target, 'runtime-manifest.json')
if (!existsSync(manifest)) throw new Error(`ORT manifest missing: ${manifest}`)
copyFileSync(manifest, path.join(output, `ort-runtime-manifest_${version}_${target}.json`))
