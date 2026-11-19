import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, readdirSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'

const licenseName = /^(?:licen[sc]es?|copying|unlicen[sc]e|ofl)(?:$|[._ -])/i
const noticeName = /(?:^|[._ -])(?:licen[sc]es?|copying|copyright|notices?|unlicen[sc]e|ofl|authors?|patents?)(?:$|[._ -])/i
const authoredPackages = new Set(['super-paste-desktop', 'super-paste-core', 'super-paste-platform', 'super-paste-format'])
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0
const slash = value => value.split(path.sep).join('/')
const digest = bytes => createHash('sha256').update(bytes).digest('hex')

function within(directory, file) {
  const relative = path.relative(directory, file)
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`))
}

function materials(directory) {
  const found = []
  let primary = false
  function visit(current, licenseDirectory = false) {
    for (const entry of readdirSync(current, { withFileTypes: true }).sort((a, b) => compare(a.name, b.name))) {
      const file = path.join(current, entry.name)
      // Dependency source may contain test/vendor licenses below its root.
      // Never descend into symlinks, build outputs or checkout metadata.
      if (entry.isDirectory() && !['.git', 'target', 'node_modules'].includes(entry.name)) {
        visit(file, licenseDirectory || (current === directory && licenseName.test(entry.name)))
      } else if (entry.isFile() && (licenseDirectory || noticeName.test(entry.name))) {
        found.push(file)
        if (licenseDirectory || (current === directory && licenseName.test(entry.name))) primary = true
      }
    }
  }
  visit(directory)
  return { files: found, primary }
}

function dependencyClosure(metadata, desktop) {
  if (metadata.version !== 1 || !metadata.resolve || !Array.isArray(metadata.resolve.nodes)) {
    throw new Error('Dependency notices require Cargo metadata format 1 with a resolved graph')
  }
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]))
  const packages = new Map(metadata.packages.map(pkg => [pkg.id, pkg]))
  const pending = [desktop.id], seen = new Set(), selected = []
  while (pending.length) {
    const id = pending.pop()
    if (seen.has(id)) continue
    seen.add(id)
    const pkg = packages.get(id), node = nodes.get(id)
    if (!pkg || !node || !Array.isArray(node.deps)) throw new Error(`Incomplete Cargo dependency graph: ${pkg?.name ?? 'unknown package'}`)
    selected.push(pkg)
    for (const dependency of node.deps) {
      if (!Array.isArray(dependency.dep_kinds) || dependency.dep_kinds.length === 0) {
        throw new Error(`Cargo dependency kind is missing for ${pkg.name}`)
      }
      for (const kind of dependency.dep_kinds) {
        if (![null, 'build', 'dev'].includes(kind.kind)) throw new Error(`Unknown Cargo dependency kind for ${pkg.name}`)
      }
      // Include normal/build dependencies and proc macros, not dev-only roots.
      // --filter-platform has already restricted this graph to the package target.
      if (dependency.dep_kinds.some(kind => kind.kind !== 'dev')) pending.push(dependency.pkg)
    }
  }
  return selected
}

function supplementalMaterials(project, pkg, supplements) {
  const entry = supplements.get(`${pkg.name}\0${pkg.version}`)
  if (!entry) return []
  if (entry.source !== pkg.source || entry.license !== pkg.license
    || entry.manifestSha256 !== digest(readFileSync(pkg.manifest_path))) {
    throw new Error(`Supplemental notices do not match the resolved dependency: ${pkg.name} ${pkg.version}`)
  }
  const base = realpathSync(path.join(project, 'packaging', 'dependency-notices'))
  if (!Array.isArray(entry.materials) || entry.materials.length === 0) {
    throw new Error(`Supplemental notice materials are missing: ${pkg.name} ${pkg.version}`)
  }
  return entry.materials.map(material => {
    if (!['license', 'notice'].includes(material.kind) || typeof material.file !== 'string'
      || typeof material.origin !== 'string' || !material.origin.startsWith('https://')) {
      throw new Error(`Invalid supplemental notice material: ${pkg.name} ${pkg.version}`)
    }
    const file = realpathSync(path.resolve(base, material.file))
    if (!within(base, file) || !statSync(file).isFile()
      || digest(readFileSync(file)) !== material.sha256) {
      throw new Error(`Supplemental notice integrity mismatch: ${pkg.name} ${pkg.version}`)
    }
    return { file, origin: material.origin, kind: material.kind, sourceFile: `supplemental/${slash(path.relative(base, file))}` }
  })
}

/** Collect the target-filtered resolve closure, not an assertion about binary
 * linkage. Cargo's workspace feature resolution can conservatively include
 * extra packages; system libraries and separately staged ORT keep their notices.
 * Package IDs are opaque. Local absolute Cargo/source paths never enter output.
 */
export function stageDependencyNotices({ metadata, desktop, root, target, directory }) {
  const project = realpathSync(root)
  const supplementIndex = JSON.parse(readFileSync(path.join(project, 'packaging', 'dependency-notices', 'index.json'), 'utf8'))
  if (supplementIndex.formatVersion !== 1 || !Array.isArray(supplementIndex.packages)) {
    throw new Error('Unsupported supplemental dependency notice index')
  }
  const supplements = new Map()
  for (const entry of supplementIndex.packages) {
    const key = `${entry.name}\0${entry.version}`
    if (supplements.has(key)) throw new Error(`Duplicate supplemental dependency notices: ${entry.name} ${entry.version}`)
    supplements.set(key, entry)
  }
  const destination = path.join(directory, 'licenses', 'rust')
  const texts = path.join(destination, 'texts')
  mkdirSync(texts, { recursive: true })
  const records = []
  for (const pkg of dependencyClosure(metadata, desktop)) {
    const sourceRoot = realpathSync(path.dirname(pkg.manifest_path))
    const local = pkg.source === null
    if (local && !within(project, sourceRoot)) throw new Error(`Local dependency escapes the independent project: ${pkg.name}`)
    if (!pkg.license && !pkg.license_file) throw new Error(`Dependency has no license declaration: ${pkg.name} ${pkg.version}`)
    const found = materials(sourceRoot)
    const files = new Set(found.files)
    let primary = found.primary
    if (pkg.license_file) {
      const declared = path.resolve(sourceRoot, pkg.license_file)
      if (!existsSync(declared) || !statSync(declared).isFile()) throw new Error(`Declared license file is missing: ${pkg.name} ${pkg.version}`)
      files.add(declared)
      primary = true
    }
    if (!primary && local && authoredPackages.has(pkg.name)
      && (within(path.join(project, 'apps'), sourceRoot) || within(path.join(project, 'crates'), sourceRoot))) {
      files.add(path.join(project, 'LICENSE'))
      primary = true
    }
    const origins = new Map()
    if (!primary) {
      for (const material of supplementalMaterials(project, pkg, supplements)) {
        files.add(material.file)
        origins.set(material.file, material)
        primary ||= material.kind === 'license'
      }
    }
    if (!primary) throw new Error(`Dependency license text is missing: ${pkg.name} ${pkg.version}`)
    const notices = []
    for (const file of [...files].sort(compare)) {
      const bytes = readFileSync(file)
      if (bytes.length === 0) throw new Error(`Dependency notice is empty: ${pkg.name} ${pkg.version} / ${path.basename(file)}`)
      const sha256 = digest(bytes)
      const stored = path.join(texts, `${sha256}.txt`)
      if (!existsSync(stored)) writeFileSync(stored, bytes)
      notices.push({
        sourceFile: origins.get(file)?.sourceFile ?? (within(sourceRoot, file) ? slash(path.relative(sourceRoot, file)) : `declared/${path.basename(file)}`),
        file: `texts/${sha256}.txt`, sha256,
        origin: origins.get(file)?.origin
      })
    }
    const source = local ? `local:${slash(path.relative(project, sourceRoot))}` : pkg.source
    records.push({
      name: pkg.name, version: pkg.version, source, license: pkg.license, notices,
      supplementalNoticeBasis: origins.size ? supplements.get(`${pkg.name}\0${pkg.version}`).basis : undefined
    })
  }
  records.sort((a, b) => compare(a.name, b.name) || compare(a.version, b.version) || compare(a.source, b.source))
  writeFileSync(path.join(destination, 'index.json'), JSON.stringify({
    formatVersion: 1, target, application: { name: desktop.name, version: desktop.version },
    scope: 'Target-filtered Cargo resolve closure of normal and build dependencies; workspace feature resolution may include additional packages. Not a binary linkage inventory.',
    packages: records
  }, null, 2) + '\n')
  const notice = [
    'Super Paste — Rust dependency notices',
    `Target: ${target}`, '',
    'Collected from the target-filtered Cargo resolve graph, including build dependencies.',
    'Workspace feature resolution may include additional packages. This is not a binary linkage inventory.',
    'License expressions are retained as declared. Supplemental materials record their explicit basis separately.',
    'Original material bytes are stored in texts/ by SHA-256; identical files are shared.',
    'ORT, copied resources and system-library declarations remain in the other package notices.', ''
  ]
  for (const pkg of records) {
    notice.push(`${pkg.name} ${pkg.version}`, `Source: ${pkg.source}`, `License: ${pkg.license ?? 'See declared license file'}`)
    if (pkg.supplementalNoticeBasis) notice.push(`Supplemental materials: ${pkg.supplementalNoticeBasis}`)
    for (const entry of pkg.notices) {
      notice.push(`  ${entry.sourceFile} -> ${entry.file}`)
      if (entry.origin) notice.push(`    Origin: ${entry.origin}`)
    }
    notice.push('')
  }
  writeFileSync(path.join(destination, 'NOTICE.txt'), notice.join('\n'))
}
