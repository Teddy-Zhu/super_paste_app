const fs = require('node:fs')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

if (process.platform !== 'linux') process.exit(0)

const targets = { x64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' }
const target = process.env.CARGO_BUILD_TARGET || targets[process.arch]
const arch = Object.keys(targets).find((arch) => targets[arch] === target)
if (!arch) throw new Error(`Unsupported KWin native target: ${target}`)

const packageDir = path.resolve(__dirname, '..')
const result = spawnSync('cargo', ['build', '--locked', '--release', '--target', target], {
  cwd: packageDir,
  stdio: 'inherit'
})
if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status || 1)

const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(packageDir, process.env.CARGO_TARGET_DIR)
  : path.join(packageDir, 'target')
fs.copyFileSync(
  path.join(targetDir, target, 'release', 'libactive_window_kwin.so'),
  path.join(packageDir, `active_window_kwin.linux-${arch}-gnu.node`)
)
