# SuperPaste 构建与打包指南

## 环境要求

### 通用依赖
- **Rust** >= 1.70 (通过 [rustup](https://rustup.rs/) 安装)
- **Node.js** >= 18 (推荐使用 [nvm](https://github.com/nvm-sh/nvm))
- **pnpm** >= 8 (通过 `npm install -g pnpm` 安装)

### 平台特定依赖

#### Linux
```bash
# Debian/Ubuntu
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev \
  build-essential \
  curl \
  wget \
  file \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev

# Arch Linux
sudo pacman -S webkit2gtk-4.1 base-devel curl wget file openssl gtk3 libappindicator-gtk3 librsvg

# Fedora
sudo dnf install webkit2gtk4.1-devel \
  openssl-devel \
  curl \
  wget \
  file \
  libappindicator-gtk3-devel \
  librsvg2-devel
sudo dnf group install "C Development Tools and Libraries"
```

#### macOS
```bash
# 安装 Xcode Command Line Tools
xcode-select --install

# 无需额外依赖
```

#### Windows
- **Microsoft Visual Studio C++ Build Tools** (通过 [Visual Studio Installer](https://visualstudio.microsoft.com/downloads/) 安装)
- **WebView2** (Windows 11 自带，Windows 10 需手动安装)

---

## 开发模式

```bash
cd super_paste_rs

# 安装依赖
pnpm install

# 启动开发服务器（热重载）
pnpm tauri dev
```

---

## 生产构建

### 方式 1：使用构建脚本（推荐）

**Linux/macOS:**
```bash
./build.sh
```

**Windows:**
```cmd
build.bat
```

### 方式 2：手动构建

```bash
# 1. 安装前端依赖
pnpm install

# 2. 构建前端
pnpm build

# 3. 构建 Tauri 应用
cd src-tauri
cargo tauri build
```

---

## 构建产物位置

构建完成后，安装包位于 `src-tauri/target/release/bundle/` 目录：

### Linux
- **AppImage**: `appimage/super_paste_rs_0.1.0_amd64.AppImage`
- **Deb**: `deb/super-paste-rs_0.1.0_amd64.deb`

### macOS
- **DMG**: `dmg/SuperPaste_0.1.0_x64.dmg`
- **App Bundle**: `macos/SuperPaste.app`

### Windows
- **MSI (WiX)**: `msi/SuperPaste_0.1.0_x64_en-US.msi`
- **NSIS**: `nsis/SuperPaste_0.1.0_x64-setup.exe`

---

## 平台特定配置

### macOS 代码签名（可选）

如需分发到 App Store 或启用 Gatekeeper，需配置代码签名：

1. 在 `tauri.conf.json` 中设置 `bundle.macOS.signingIdentity`
2. 配置 entitlements（如需沙盒）

```json
"macOS": {
  "signingIdentity": "Developer ID Application: Your Name (TEAM_ID)",
  "entitlements": "entitlements.plist"
}
```

### Windows 代码签名（可选）

如需避免 SmartScreen 警告，需配置代码签名：

```json
"windows": {
  "certificateThumbprint": "YOUR_CERT_THUMBPRINT",
  "timestampUrl": "http://timestamp.digicert.com"
}
```

---

## CI/CD 构建

### GitHub Actions 示例

```yaml
name: Build

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    strategy:
      matrix:
        platform: [ubuntu-22.04, macos-latest, windows-latest]
    runs-on: ${{ matrix.platform }}

    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - uses: pnpm/action-setup@v2
        with:
          version: 8
      - uses: dtolnay/rust-toolchain@stable

      - name: Install Linux dependencies
        if: matrix.platform == 'ubuntu-22.04'
        run: |
          sudo apt update
          sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev

      - name: Build
        run: |
          pnpm install
          pnpm tauri build

      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.platform }}
          path: src-tauri/target/release/bundle/
```

---

## 故障排查

### 构建失败：缺少依赖

**Linux**: 确保安装了所有 webkit2gtk 和 GTK 开发库  
**Windows**: 确保安装了 Visual Studio C++ Build Tools  
**macOS**: 运行 `xcode-select --install`

### 前端构建失败

```bash
# 清理缓存并重新安装
rm -rf node_modules pnpm-lock.yaml
pnpm install
```

### Rust 编译错误

```bash
# 更新 Rust 工具链
rustup update stable

# 清理构建缓存
cd src-tauri
cargo clean
```

---

## 性能优化

### 减小二进制体积

在 `src-tauri/Cargo.toml` 中启用 LTO 和 strip：

```toml
[profile.release]
lto = true
strip = true
opt-level = "z"
codegen-units = 1
```

### 加速构建

```bash
# 使用 sccache 缓存编译结果
cargo install sccache
export RUSTC_WRAPPER=sccache
```

---

## 更多信息

- [Tauri 官方文档](https://tauri.app/v2/guides/)
- [Tauri 打包指南](https://tauri.app/v2/guides/building/)
- [项目实现文档](../docs/tauri-migration-implementation-plan.md)
