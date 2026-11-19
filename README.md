# Super Paste

Super Paste 是一款使用 Rust 和 GPUI Kit 开发的桌面剪贴板管理器，适用于 Windows、macOS 和 Linux。它在本机保存剪贴板历史，支持搜索、预览和重新复制已保存的内容。

[下载安装包](https://github.com/Teddy-Zhu/super_paste_app/releases) · [问题反馈](https://github.com/Teddy-Zhu/super_paste_app/issues)

## 功能

- 记录和搜索剪贴板历史，通过标签整理记录。
- 预览文本、HTML、RTF、图片和文件，在独立窗口查看详情。
- 将历史记录按原格式或纯文本复制回剪贴板。
- 使用全局快捷键或系统托盘打开粘贴面板。
- 从图片识别文字，生成和识别二维码，查看使用统计。
- 通过 WebDAV 同步设置，通过 MCP 服务接入兼容客户端。

## 安装

从下载页选择对应平台和架构的安装包。下表列出当前源码提供的构建目标和包格式。

| 平台 | 架构与系统要求 | 安装格式 |
| --- | --- | --- |
| Windows | x86_64，需安装 Microsoft Visual C++ x64 运行库 | `.exe` 安装程序、便携 `.zip` |
| macOS | Intel 或 Apple Silicon，macOS 13.4 及以上 | `.dmg`、`.app.tar.gz` |
| Linux | x86_64，支持 X11 和 KDE Plasma 6 Wayland | `.deb`、`.AppImage`、便携 `.tar.xz`、Arch `.pkg.tar.zst` |

Arch Linux 可通过 AUR 安装。 [super-paste-bin](https://aur.archlinux.org/packages/super-paste-bin)

运行 AppImage 前需赋予执行权限。使用便携包时，请完整解压并保留可执行文件旁的 `resources/` 目录。其他 Wayland 合成器上的剪贴板和全局快捷键功能取决于桌面环境提供的接口。

## 快速开始

1. 启动 Super Paste，在设置的「快捷键」页面录制全局快捷键。也可以从系统托盘或 macOS 菜单栏打开粘贴面板。
2. 在其他应用中复制内容，再打开粘贴面板浏览或搜索历史记录。
3. 选择历史记录，将内容写回系统剪贴板，然后在目标应用中粘贴。

| 命令 | 用途 |
| --- | --- |
| `super-paste --toggle-overlay` | 打开或关闭粘贴面板 |
| `super-paste --background` | 后台启动 |

## 数据与隐私

剪贴板历史存储在本机 SQLite 数据库。WebDAV 的同步范围限于非敏感设置，访问凭据保存在系统凭据库。

本地 OCR 模型可在「OCR 设置」中按需下载。使用远程 OCR 时，待识别图像会发送至配置的服务。

| 平台 | 默认数据目录 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\com.v2dawn.superpaste.rs` |
| macOS | `~/Library/Application Support/com.v2dawn.superpaste.rs` |
| Linux | `$XDG_DATA_HOME/com.v2dawn.superpaste.rs`，默认使用 `~/.local/share` |

## 从源码构建

需要 Rustup 和 Node.js 20 或更高版本。Rust 工具链由 `rust-toolchain.toml` 指定。

| 平台 | 编译依赖 |
| --- | --- |
| Windows | Visual Studio C++ 生成工具和 Windows SDK。在 x64 MSVC 开发者终端中构建，并确保 `rc.exe` 和 `fxc.exe` 可用。 |
| macOS | Xcode 和 Metal 工具链。在与目标架构一致的主机上构建。 |
| Linux | C/C++ 工具链、CMake、Clang、pkg-config、unzip，以及 GTK 3、Fontconfig、FreeType、X11/XCB、xkbcommon、Wayland、EGL 和 Vulkan 开发库。 |

在仓库根目录运行以下命令。第一个命令准备并校验 ONNX Runtime。

```sh
node scripts/prepare-ort.mjs
cargo run --locked -p super-paste-desktop --bin super-paste
```

生成本机安装包。

```sh
cargo install cargo-packager --version 0.11.8 --locked
node scripts/build.mjs
```

产物位于 `dist/<target>/<version>/`。可通过 `node scripts/build.mjs --help` 查看打包选项。Linux 的 DEB 和便携包需要 binutils 与 XZ，Arch 包需要 `makepkg` 和 `fakeroot`。

## 参与贡献

欢迎提交 Issue 和 Pull Request。报告问题时请附上应用版本、操作系统和复现步骤。Linux 桌面问题请注明桌面环境及 X11 或 Wayland 会话类型。

[开发规范](AGENTS.md) · [实现方案](docs/IMPLEMENTATION_PLAN.md)

## 许可证

Super Paste 使用 MIT 许可证。 [许可证全文](LICENSE) · [第三方声明](THIRD-PARTY-NOTICES.md)
