const fs = require('fs');
const path = require('path');

module.exports = async function (context) {
  const unpackedDir = path.join(context.appOutDir, 'resources', 'app.asar.unpacked');
  const napiBinBase = path.join(unpackedDir, 'node_modules', 'onnxruntime-node', 'bin', 'napi-v6');
  console.log(`[afterPack] napiBinBase: ${napiBinBase}`);
  // 从 electron-builder 传入的 context 获取目标平台
  const targetPlatform = context.electronPlatformName; // 'linux', 'darwin', 'win32'
  // const targetArch = context.arch; // 'x64', 'arm64', etc.
  const archMap = {
    0: 'ia32',
    1: 'x64',
    2: 'armv7l',
    3: 'arm64'
  };

  const targetArch = archMap[context.arch];
  console.log(`[afterPack] Keeping only napi-v6/${targetPlatform}/${targetArch} binaries`);

  if (!fs.existsSync(napiBinBase)) {
    console.warn(`[afterPack] Path not found: ${napiBinBase}`);
    return;
  }

  const platforms = fs.readdirSync(napiBinBase);
  for (const platform of platforms) {
    const platformPath = path.join(napiBinBase, platform);
    if (!fs.statSync(platformPath).isDirectory()) continue;

    const archs = fs.readdirSync(platformPath);
    for (const arch of archs) {
      if (platform !== targetPlatform || arch !== targetArch) {
        const fullPath = path.join(platformPath, arch);
        console.log(`[afterPack] Removing unused onnxruntime binary: ${fullPath}`);
        fs.rmSync(fullPath, { recursive: true, force: true });
      }
    }

    const remaining = fs.readdirSync(platformPath);
    if (remaining.length === 0) {
      console.log(`[afterPack] Removing empty platform folder: ${platformPath}`);
      fs.rmSync(platformPath, { recursive: true, force: true });
    }
  }
};
