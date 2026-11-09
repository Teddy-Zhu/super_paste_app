import { execFileSync } from 'node:child_process'
import { createRequire } from 'node:module'
import path from 'node:path'
import { githubRepository, productName, projectRoot, version } from './project.mjs'

const require = createRequire(import.meta.url)

function run(command, args, quiet = false) {
  execFileSync(command, args, { cwd: projectRoot, stdio: quiet ? 'ignore' : 'inherit' })
}

function runScript(name, args = []) {
  run(process.execPath, [path.join(projectRoot, 'scripts', name), ...args])
}

function optionValue(args, names) {
  const index = args.findIndex((arg) =>
    names.some((name) => arg === name || arg.startsWith(`${name}=`))
  )
  if (index < 0) return undefined
  const arg = args[index]
  const value = arg.includes('=') ? arg.slice(arg.indexOf('=') + 1) : args[index + 1]
  if (!value || value.startsWith('-')) throw new Error(`${arg} requires a value`)
  return { index, value, count: arg.includes('=') ? 1 : 2 }
}

function main() {
  const args = process.argv.slice(2)
  if (args[0] === '--') args.shift()
  if (args.includes('--help') || args.includes('-h')) {
    console.log(
      `Build ${productName}\n\n  pnpm build                     Build platform installers\n  pnpm build --bundles deb        Build a Debian package\n  pnpm build:pacman               Build a local Pacman package and AUR metadata\n\nOther options are passed to 'tauri build'. Pacman packaging requires Arch Linux makepkg.`
    )
    return
  }
  const target = optionValue(args, ['--target', '-t'])?.value
  const bundles = optionValue(args, ['--bundles', '-b'])
  const formats = bundles?.value.split(',') ?? []
  const pacman = formats.includes('pacman')
  let repository
  if (pacman) {
    if (
      process.platform !== 'linux' ||
      process.arch !== 'x64' ||
      (target && target !== 'x86_64-unknown-linux-gnu')
    ) {
      throw new Error('Pacman packaging requires an x86_64 Linux host and target')
    }
    if (args.some((arg) => ['--debug', '-d', '--no-bundle'].includes(arg))) {
      throw new Error('Pacman packaging requires a release build with bundling enabled')
    }
    repository = githubRepository()
    run('makepkg', ['--version'], true)
    run('bsdtar', ['--version'], true)
    const nativeFormats = [...new Set([...formats.filter((format) => format !== 'pacman'), 'deb'])]
    args.splice(
      bundles.index,
      bundles.count,
      '--bundles',
      nativeFormats.includes('all') ? 'all' : nativeFormats.join(',')
    )
  }

  console.log(`Building ${productName} ${version}`)
  runScript('prepare-ort.mjs', target ? ['--target', target] : [])
  run(process.execPath, [require.resolve('@tauri-apps/cli/tauri.js'), 'build', ...args])
  if (pacman) {
    const deb = path.join(
      projectRoot,
      'target',
      ...(target ? [target] : []),
      'release',
      'bundle',
      'deb',
      `${productName}_${version}_amd64.deb`
    )
    runScript('prepare-aur.mjs', ['--repository', repository, '--deb', deb])
  }
}

try {
  main()
} catch (error) {
  console.error(`Build failed: ${error.message}`)
  process.exitCode = typeof error.status === 'number' && error.status > 0 ? error.status : 1
}
