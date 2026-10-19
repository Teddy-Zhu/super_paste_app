import { readFile, readdir } from 'node:fs/promises'
import { homedir } from 'node:os'
import { basename, extname, isAbsolute, join, relative } from 'node:path'
import logger from '../utils/logger'

type Ini = Map<string, Map<string, string>>
interface DesktopEntry {
  id: string
  fields: Map<string, string>
}
interface IconTheme {
  name: string
  directories: string[]
}

const dataDirectories = [
  process.env.XDG_DATA_HOME || join(homedir(), '.local/share'),
  ...(process.env.XDG_DATA_DIRS || '/usr/local/share:/usr/share').split(':')
].filter(isAbsolute)
const iconDirectories = [
  ...new Set([join(homedir(), '.icons'), ...dataDirectories.map((p) => join(p, 'icons'))])
]
const iconCache = new Map<string, Promise<string>>()
const iniEscapes: Record<string, string> = { s: ' ', n: '\n', t: '\t', r: '\r', '\\': '\\' }
const iconMimeTypes: Record<string, string> = { '.png': 'image/png', '.svg': 'image/svg+xml' }
let desktopEntries: Promise<DesktopEntry[]> | undefined
let iconThemes: Promise<IconTheme[]> | undefined

function parseIni(text: string): Ini {
  const sections: Ini = new Map()
  let section: Map<string, string> | undefined
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#')) continue
    if (line.startsWith('[') && line.endsWith(']')) {
      section = new Map()
      sections.set(line.slice(1, -1), section)
    } else if (section) {
      const separator = line.indexOf('=')
      if (separator > 0) {
        const value = line
          .slice(separator + 1)
          .trim()
          .replace(/\\([sntr\\])/g, (_, escaped: string) => {
            return iniEscapes[escaped]
          })
        section.set(line.slice(0, separator).trim(), value)
      }
    }
  }
  return sections
}

function isMissing(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException).code
  return code === 'ENOENT' || code === 'ENOTDIR'
}

async function readOptional(path: string): Promise<Buffer | undefined> {
  try {
    return await readFile(path)
  } catch (error) {
    if (isMissing(error)) return undefined
    throw error
  }
}

async function loadDesktopEntries(): Promise<DesktopEntry[]> {
  const entries = new Map<string, DesktopEntry>()
  for (const dataDirectory of dataDirectories) {
    const directory = join(dataDirectory, 'applications')
    let files
    try {
      files = await readdir(directory, { withFileTypes: true, recursive: true })
    } catch (error) {
      if (isMissing(error)) continue
      throw error
    }
    const candidates = await Promise.all(
      files
        .filter((file) => !file.isDirectory() && file.name.endsWith('.desktop'))
        .map(async (file) => {
          const path = join(file.parentPath, file.name)
          const data = await readOptional(path)
          const fields = data && parseIni(data.toString()).get('Desktop Entry')
          if (!fields) return undefined
          return { id: relative(directory, path).replaceAll('/', '-').slice(0, -8), fields }
        })
    )
    // A user entry, including Hidden=true, overrides the corresponding system entry.
    for (const entry of candidates) {
      if (entry && !entries.has(entry.id)) entries.set(entry.id, entry)
    }
  }
  return [...entries.values()].filter((entry) => entry.fields.get('Hidden') !== 'true')
}

function sizeDistance(fields: Map<string, string>): number {
  const size = Number(fields.get('Size'))
  if (!size) return Infinity
  const scale = Number(fields.get('Scale') || 1)
  const type = fields.get('Type') || 'Threshold'
  const threshold = type === 'Threshold' ? Number(fields.get('Threshold') || 2) : 0
  const min = type === 'Scalable' ? Number(fields.get('MinSize') || size) : size - threshold
  const max = type === 'Scalable' ? Number(fields.get('MaxSize') || size) : size + threshold
  return Math.max(min * scale - 64, 64 - max * scale, 0)
}

async function loadIconThemes(): Promise<IconTheme[]> {
  const configDirectory = process.env.XDG_CONFIG_HOME || join(homedir(), '.config')
  const config = await readOptional(join(configDirectory, 'kdeglobals'))
  const selected = config && parseIni(config.toString()).get('Icons')?.get('Theme')
  const themes: IconTheme[] = []
  const visited = new Set<string>()
  const visit = async (name: string): Promise<void> => {
    if (visited.has(name) || !name || name.includes('/') || name === '..') return
    visited.add(name)
    for (const root of iconDirectories) {
      const data = await readOptional(join(root, name, 'index.theme'))
      if (!data) continue
      const sections = parseIni(data.toString())
      const theme = sections.get('Icon Theme')
      if (!theme) continue
      const directories = [theme.get('Directories'), theme.get('ScaledDirectories')]
        .flatMap((value) => value?.split(',') || [])
        .map((value) => value.trim())
        .filter(Boolean)
        .sort((a, b) => {
          const first = sections.get(a) || new Map<string, string>()
          const second = sections.get(b) || new Map<string, string>()
          // Application icons normally live in Apps directories; avoid probing every action icon first.
          const context =
            Number(second.get('Context') === 'Applications') -
            Number(first.get('Context') === 'Applications')
          return context || sizeDistance(first) - sizeDistance(second)
        })
      themes.push({ name, directories: [...new Set(directories)] })
      for (const parent of theme.get('Inherits')?.split(',') || []) {
        if (parent.trim() !== 'hicolor') await visit(parent.trim())
      }
      break
    }
  }
  if (selected && selected !== 'hicolor') await visit(selected)
  await visit('hicolor')
  return themes
}

async function readIcon(path: string): Promise<string> {
  const mime = iconMimeTypes[extname(path)]
  if (!mime) return ''
  const data = await readOptional(path)
  return data?.length ? `data:${mime};base64,${data.toString('base64')}` : ''
}

async function findIcon(directories: string[], fileNames: string[]): Promise<string> {
  for (const directory of directories) {
    for (const fileName of fileNames) {
      const icon = await readIcon(join(directory, fileName))
      if (icon) return icon
    }
  }
  return ''
}

async function resolveIcon(application: string, executable: string): Promise<string> {
  desktopEntries ||= loadDesktopEntries()
  const entries = await desktopEntries
  const id = application.replace(/\.desktop$/, '').toLowerCase()
  const executableName = executable ? basename(executable) : ''
  let selected: DesktopEntry | undefined
  let bestScore = 0
  for (const entry of entries) {
    const command = entry.fields.get('Exec')?.match(/^(?:"([^"]+)"|(\S+))/)
    const score =
      id && entry.id.toLowerCase() === id
        ? 3
        : id && entry.fields.get('StartupWMClass')?.toLowerCase() === id
          ? 2
          : executableName && command && basename(command[1] || command[2]) === executableName
            ? 1
            : 0
    if (score > bestScore) {
      selected = entry
      bestScore = score
    }
  }
  const name = selected?.fields.get('Icon')
  if (!name) {
    logger.debug('[ActiveWindowService] Linux icon unavailable', {
      application,
      desktopId: selected?.id,
      reason: 'no-desktop-icon'
    })
    return ''
  }
  let icon = ''
  if (isAbsolute(name)) {
    icon = await readIcon(name)
  } else if (!name.includes('/')) {
    iconThemes ||= loadIconThemes()
    const fileNames = /\.(png|svg)$/.test(name) ? [name] : [`${name}.png`, `${name}.svg`]
    for (const theme of await iconThemes) {
      const directories = theme.directories.flatMap((directory) =>
        iconDirectories.map((root) => join(root, theme.name, directory))
      )
      icon = await findIcon(directories, fileNames)
      if (icon) break
    }
    if (!icon) {
      icon = await findIcon(
        [...iconDirectories, ...dataDirectories.map((p) => join(p, 'pixmaps'))],
        fileNames
      )
    }
  }
  logger.debug('[ActiveWindowService] Linux icon resolved', {
    application,
    desktopId: selected?.id,
    iconName: name,
    hasIcon: Boolean(icon)
  })
  return icon
}

/** Resolve the application's desktop icon, rather than the executable's MIME icon. */
export function getLinuxAppIcon(application: string, executable: string): Promise<string> {
  const key = `${application}\0${executable}`
  let icon = iconCache.get(key)
  if (!icon) {
    icon = resolveIcon(application, executable).catch((error: unknown) => {
      logger.warn('[ActiveWindowService] Linux icon lookup failed', {
        application,
        error: error instanceof Error ? error.message : String(error)
      })
      return ''
    })
    if (iconCache.size >= 64) iconCache.delete(iconCache.keys().next().value!)
    iconCache.set(key, icon)
  }
  return icon
}
