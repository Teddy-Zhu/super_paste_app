import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readFileSync, readSync, readdirSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { sha256File } from './file-hash.mjs'

// Resolve materials from the packages that supplied the deployed libraries.
// Keep the original declaration and bytes; do not infer a replacement license.
export function stageNativeNotices(appDir, application, run) {
  const manager = existsSync('/var/lib/pacman/local') ? 'pacman'
    : existsSync('/var/lib/dpkg/status') ? 'dpkg-query' : null
  if (!manager) throw new Error('AppImage native notices require an Arch or Debian package database on the build host')
  const query = args => run(manager, args, { capture: true, env: { LC_ALL: 'C' } }).trim()
  const destination = path.join(appDir, 'usr', 'share', 'licenses', `${application}-native`)
  const texts = path.join(destination, 'texts')
  mkdirSync(texts, { recursive: true })
  const packages = new Map()
  const materials = new Map()
  const store = file => {
    const source = realpathSync(file)
    if (!statSync(source).isFile()) throw new Error(`Native license material is not a file: ${source}`)
    if (!materials.has(source)) {
      const sha256 = sha256File(source)
      if (!statSync(source).size) throw new Error(`Native license material is empty: ${source}`)
      const name = `${sha256}.txt`
      if (!existsSync(path.join(texts, name))) copyFileSync(source, path.join(texts, name))
      materials.set(source, { source, file: `texts/${name}`, sha256 })
    }
    return materials.get(source)
  }
  const libraryDir = path.join(appDir, 'usr', 'lib')
  const libraries = []
  for (const entry of readdirSync(libraryDir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isFile()) continue
    const library = path.join(libraryDir, entry.name)
    const header = Buffer.alloc(4)
    const fd = openSync(library, 'r')
    try { readSync(fd, header, 0, header.length, 0) } finally { closeSync(fd) }
    if (!header.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) continue
    const host = ['/usr/lib', '/lib', '/usr/lib/x86_64-linux-gnu', '/lib/x86_64-linux-gnu']
      .map(directory => path.join(directory, entry.name)).find(file => existsSync(file) && statSync(file).isFile())
    if (!host) throw new Error(`Cannot resolve AppImage library to a host package: ${entry.name}`)
    // usr-merge aliases can differ from Debian's recorded pathname. Resolve
    // matching package paths to the actual file before choosing its owner.
    const owners = manager === 'pacman' ? [query(['-Qqo', host])] : query(['-S', `*/${entry.name}`]).split('\n')
      .map(line => line.split(': ')).filter(parts => parts.length === 2 && existsSync(parts[1]) && realpathSync(parts[1]) === realpathSync(host))
      .map(parts => parts[0])
    const owner = [...new Set(owners)].join(',')
    if (!owner || owner.includes('\n') || owner.includes(',')) throw new Error(`Ambiguous native package owner: ${host}`)
    if (!packages.has(owner)) {
      const files = new Set()
      let version, url, declaration = null
      if (manager === 'pacman') {
        const info = Object.fromEntries(query(['-Qi', owner]).split('\n')
          .map(line => line.match(/^([^ :][^:]+?)\s+:\s*(.*)$/)).filter(Boolean).map(match => [match[1].trim(), match[2]]))
        version = info.Version; url = info.URL; declaration = info.Licenses
        if (!declaration || declaration === 'None') throw new Error(`Missing native license declaration: ${owner}`)
        const visited = new Set()
        const collect = directory => {
          const resolved = realpathSync(directory)
          if (visited.has(resolved)) return
          visited.add(resolved)
          for (const name of readdirSync(directory).sort()) {
            const file = path.join(directory, name)
            if (statSync(file).isDirectory()) collect(file)
            else files.add(file)
          }
        }
        const directory = path.join('/usr/share/licenses', owner)
        if (existsSync(directory)) collect(directory)
        for (const token of declaration.match(/[A-Za-z0-9][A-Za-z0-9.+-]*/g) ?? []) {
          if (['AND', 'OR', 'WITH'].includes(token)) continue
          for (const file of [`/usr/share/licenses/spdx/${token}.txt`, `/usr/share/licenses/spdx/exceptions/${token}.txt`, `/usr/share/licenses/common/${token}`]) {
            if (existsSync(file) && statSync(file).isFile()) files.add(file)
          }
        }
      } else {
        const values = query(['-W', '-f=${Version}\n${Homepage}', owner]).split('\n')
        version = values[0]; url = values[1] ?? null
        const copyright = path.join('/usr/share/doc', owner.split(':')[0], 'copyright')
        if (existsSync(copyright)) files.add(copyright)
      }
      if (!version || !files.size) throw new Error(`Missing native package version or license material: ${owner}`)
      // Debian copyright files may reference shared license texts explicitly.
      for (const file of [...files]) {
        const text = readFileSync(file, 'utf8')
        for (const match of text.matchAll(/\/usr\/share\/common-licenses\/([A-Za-z0-9](?:[A-Za-z0-9.+-]*[A-Za-z0-9])?)/g)) {
          const referenced = `/usr/share/common-licenses/${match[1]}`
          if (!existsSync(referenced)) throw new Error(`Missing referenced native license: ${referenced}`)
          files.add(referenced)
        }
      }
      packages.set(owner, { name: owner, version, url, declaration, materials: [...files].sort().map(store) })
    }
    libraries.push({ file: `usr/lib/${entry.name}`, sha256: sha256File(library), source: realpathSync(host), sourceSha256: sha256File(host), package: owner })
  }
  if (!libraries.length) throw new Error('AppImage contains no deployed native libraries')
  writeFileSync(path.join(destination, 'index.json'), JSON.stringify({
    manager, libraries, packages: [...packages.values()].sort((a, b) => a.name.localeCompare(b.name))
  }, null, 2) + '\n')
}
