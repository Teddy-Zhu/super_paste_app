import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { projectRoot, packageFormats, releaseTargets, hostTarget } from './project.mjs'
import { preparePackageTools } from './prepare-package-tools.mjs'

try {
  if (process.argv.slice(2).join(' ') === '--help') {
    console.log('Usage: node scripts/build.mjs [--formats format,format]\nPrepares verified resources, builds the native release executable and packages it.\n' +
      Object.entries(releaseTargets).map(([target, spec]) => `${target}: ${spec.formats.join(',')}`).join('\n'))
  } else {
    if (Number(process.versions.node.split('.')[0]) < 20) throw new Error('Node.js 20+ is required')
    const formats = packageFormats(process.argv.slice(2))
    console.log(`Building ${hostTarget()}: ${formats.join(',')}`)
    const env = await preparePackageTools(formats)
    const result = spawnSync(process.execPath, [path.join(projectRoot, 'scripts/package.mjs'), '--formats', formats.join(',')], {
      cwd: projectRoot, env: { ...process.env, ...env }, stdio: 'inherit'
    })
    if (result.error || result.status !== 0) throw result.error ?? new Error(`Packaging failed (${result.signal ?? result.status})`)
  }
} catch (error) { console.error(error.message); process.exitCode = 1 }
