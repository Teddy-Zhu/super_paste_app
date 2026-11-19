# Super Paste

Rust + GPUI Kit 原生桌面剪贴板管理器，当前版本 **1.0.3**。支持剪贴板历史、搜索与标签、多格式还原、富文本和图片预览、OCR、二维码、统计、WebDAV 设置同步及 MCP。

本仓库根目录现为原 `super_paste_rs` 项目；旧 Vue/Tauri 实现已移除，可从 Git 历史查阅。开发和发布均从根目录执行，无需 pnpm、前端构建或 WebView。

## 开发运行

需要 Rustup、Node.js 20+ 和平台编译依赖。Rust 版本由 `rust-toolchain.toml` 固定为 1.95.0。

```sh
node scripts/prepare-ort.mjs
cargo run --locked -p super-paste-desktop --bin super-paste
```

首次普通启动打开设置。默认全局快捷键为空，请在“快捷键”中录制，或通过托盘打开粘贴面板。已有实例运行时，普通二次启动显示设置；以下参数会转发给已有实例：

```sh
# 打开或切换粘贴面板
cargo run --locked -p super-paste-desktop --bin super-paste -- --toggle-overlay

# 后台启动
cargo run --locked -p super-paste-desktop --bin super-paste -- --background
```

`prepare-ort.mjs` 按目标下载并校验 ONNX Runtime，不下载模型权重；OCR 模型从应用设置按需下载。开发版从本仓库 `resources/` 加载资源，发布版必须携带完整安装包或便携包资源，不能只复制裸二进制。

| 平台 | 构建和运行条件 |
| --- | --- |
| Linux x86_64 GNU | C/C++ 工具链、pkg-config、Clang、字体、X11/Wayland、图形及 GTK 3 开发库；运行需要图形桌面、GPU 驱动、会话 D-Bus 和系统凭据服务 |
| Windows x86_64 | Visual Studio MSVC 开发者终端及 Windows SDK，确保 `rc.exe`、`fxc.exe` 可用；目标系统需要 Visual C++ x64 运行库 |
| macOS Intel / Apple Silicon | Xcode、Apple SDK、Metal 工具链；最低系统版本 13.3，分别在对应架构宿主构建 |

Ubuntu 开发依赖清单与 CI 保持一致，见 [Release workflow](.github/workflows/release.yml)。GTK 3 用于 Linux 托盘兼容，仍是链接依赖；无需 WebKit。Wayland 的完整支持目标为 KDE Plasma 6，其他合成器的能力以实际环境为准。

## 自动构建和打包

先安装固定版本的打包器，然后使用统一构建入口：

```sh
cargo install cargo-packager --version 0.11.8 --locked
node scripts/build.mjs
```

脚本根据宿主选择格式，准备并验证 AppImage 工具和 ORT，收集依赖许可证，执行锁定依赖的 release 编译并打包。产物写入 `dist/<target>/<version>/`；打包编译目录固定为本仓库 `target/`，不受外部 `CARGO_TARGET_DIR` 影响。

| 宿主 | 默认格式 | 可用格式 |
| --- | --- | --- |
| Linux x86_64 | DEB、AppImage、便携 tar.xz | `deb,pacman,appimage,portable` |
| Windows x86_64 | NSIS 安装程序、便携 ZIP | `nsis,portable` |
| macOS x86_64 / arm64 | DMG、App bundle | `dmg,app` |

```sh
# 仅生成便携包，无需 cargo-packager
node scripts/build.mjs --formats portable

# 仅生成 DEB
node scripts/build.mjs --formats deb

# Arch Linux 上生成全部 Linux 格式；pacman 需要 makepkg / fakeroot
node scripts/build.mjs --formats deb,pacman,appimage,portable
```

Linux 打包使用 binutils 和 XZ。便携包用 `tar -xJf <文件.tar.xz>` 解包，DEB 的安装方式照常。打包器将主程序的符号提取到输出目录的 `symbols/super-paste.debug`，GitHub Actions 将该文件留在单独的构建产物中，供发布版故障定位。

macOS 的 App bundle 位于相同输出目录，发布收集脚本会将其封装为 `.app.tar.gz`。默认不配置证书签名和公证；已有本机签名身份可通过 `SUPER_PASTE_MAC_SIGNING_IDENTITY` 传入。Windows 和 macOS 的签名、安装及完整运行验收仍需在对应环境完成。

## GitHub Actions 发布 Release

[Super Paste release](.github/workflows/release.yml) 支持两种触发方式：

- 推送与工作区版本一致的 `v<version>` 标签。
- 在 Actions 手动运行：`release_tag` 留空时发布所选 ref 的工作区版本，并在最终发布时创建标签；填写时只构建该已有标签。

版本唯一来源为根 `Cargo.toml` 的 `[workspace.package].version`，桌面程序和业务 crate 继承它。修改版本后更新并提交 `Cargo.lock`，标签必须一致。例如当前版本：

```sh
cargo update --workspace
node scripts/verify-release-tag.mjs v1.0.3
# 在代码和锁文件提交完成后创建、推送标签
git tag v1.0.3
git push origin v1.0.3
```

流程固定同一个提交，分别在 Windows、Linux、macOS Intel 和 Apple Silicon 原生 runner 构建，随后在 Arch 容器中复用 Linux DEB 生成 Pacman 包及 AUR 元数据。全部成功后创建或恢复草稿，上传资源、`SHA256SUMS` 和按提交生成的更新记录，最后公开 Release；不会覆盖已经公开的同名 Release。

发布资产包括：

- Windows NSIS 安装程序和便携 ZIP。
- Linux DEB、AppImage、便携 tar.xz、Arch `.pkg.tar.zst`。
- macOS 两种架构的 DMG 和 App bundle 压缩包。
- 各目标的 ORT 清单、`super-paste-bin.PKGBUILD`、`super-paste-bin.SRCINFO` 和总校验和。

工作流使用仓库自带 `GITHUB_TOKEN`，仅发布 job 请求 `contents: write`；仓库或组织策略需允许此权限。普通构建不需要额外发布凭据。

## AUR 发布

沿用 AUR 包名 **`super-paste-bin`**，安装 `super-paste` 程序，并提供 `super-paste`。依赖由 GPUI 打包配置生成，不再依赖 WebKit 或 Tauri。当前 `1.0.3` 可正常升级旧版 `1.x`，不增加 epoch。

在 GitHub 仓库的 Actions secrets / variables 中配置：

| 名称 | 类型 | 用途 |
| --- | --- | --- |
| `AUR_SSH_PRIVATE_KEY` | Secret，必需 | 对 `super-paste-bin` 有维护权限的 AUR SSH 私钥 |
| `AUR_GIT_NAME`、`AUR_GIT_EMAIL` | Variable，可选 | AUR 提交身份，缺省使用 GitHub Actions bot |
| `AUR_AUTO_PUBLISH` | Variable，可选 | 设为字符串 `true` 后，每次 Release 成功都自动发布 AUR |

也可手动运行 Release 并勾选 `publish_aur`，或单独运行 [Publish AUR](.github/workflows/aur.yml)，选择已经公开的 `release_tag`；留空使用最新稳定 Release。AUR 要求仓库与 Release 资产公开。

AUR 流程下载已发布的配方、元数据和 DEB，验证校验和、版本、下载地址和 `.SRCINFO` 一致性，校验 SSH 主机指纹后提交。禁止版本倒退；同版本无变化不提交，配方有变化时递增 `pkgrel`。这些操作仅在用户触发或配置的工作流中执行，本地构建不会推送。

Arch Linux 本地准备包和元数据：

```sh
node scripts/build.mjs --formats deb
node scripts/prepare-aur.mjs --repository owner/repo
```

输出在 `release-assets/`，可用 `--deb <file>`、`--output <directory>` 指定输入输出。Pacman 打包保留原始 ORT 字节，禁用再次 strip，避免破坏运行库清单校验。

## 数据与交互

- 软件名称为 Super Paste，可执行文件和发布包使用 `super-paste`。数据、凭据和单实例继续使用内部标识 `com.v2dawn.superpaste.rs`，以保留已有记录和配置；不导入旧 Tauri 数据或凭据。
- Linux 数据默认位于 `${XDG_DATA_HOME:-$HOME/.local/share}/com.v2dawn.superpaste.rs`；模型、日志位于数据目录，缓存使用系统缓存目录，凭据仅存系统凭据库。
- SQLite 分离热元数据、完整内容和原图 BLOB；预览可重建，原始多格式内容用于还原。还原只写回剪贴板，不模拟 Ctrl/Cmd+V。
- 图片、受控富文本解析和在途任务按实际依赖共享，具有预算及回收规则；二维码使用 `qrcode` / `rqrr`。
- 详情使用独立居中窗口，按内容调整尺寸；打开、点击和关闭详情均保持粘贴面板，关闭后返回原查询与浏览位置。
- WebDAV 只同步非敏感设置，不同步历史数据库或凭据。

## 工程与验收状态

| 路径 | 职责 |
| --- | --- |
| `apps/desktop/` | GPUI 启动、窗口宿主、设置与粘贴面板 |
| `crates/app-core/` | 历史、配置、图片、预览、OCR、同步和 MCP |
| `crates/data-format/` | 领域类型、配置及数据库 schema |
| `crates/desktop-platform/` | 剪贴板、托盘、快捷键、窗口及凭据接入 |
| `vendor/` | 固定依赖及必要补丁，见 [GPUI 补丁说明](vendor/GPUI-PATCHES.md) |
| `scripts/`、`packaging/`、`resources/` | 构建、发布、资源准备和打包 |

核心业务及 UI 调用已有实现，Linux KDE X11 开发版已进行局部运行调试。1.0.3 已重新构建 Linux 四种包，DEB 为 29.37 MiB、AppImage 为 58.65 MiB、便携包为 29.24 MiB、Arch 包为 35.39 MiB。各包的主程序代码段、分离符号关联和 570 项资源摘要已核对，AUR 配方使用新 DEB 的校验和。复杂富文本、完整 UI、安装包运行、KDE Wayland、Windows MSVC 与 macOS 仍需实际平台验收。远端 Actions 和线上发布尚未执行。

[实施方案与契约](docs/IMPLEMENTATION_PLAN.md) · [历史实施记录](docs/README-history-2026-11-19.md) · [开发规范](AGENTS.md) · [第三方声明](THIRD-PARTY-NOTICES.md)
