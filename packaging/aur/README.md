# Arch Linux / AUR

提供两种互斥的 x86_64 包：

| 包名              | 构建和运行方式                                                                                             |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| `super-paste-git` | 拉取 Git 源码，编译应用、SQLite、Canvas 和活动窗口模块；运行时复用系统 `electron44`，不重复安装 Electron。 |
| `super-paste-bin` | 下载 GitHub Release 的 deb，重新打包上游二进制及其 Electron；不编译代码，不执行 deb 的安装脚本。           |

Git 版需要可访问源码仓库、npm registry、crates.io、kdotool Git 仓库和 Electron headers。首次构建必须联网。Arch 衍生发行版需要在自己的仓库或 AUR 中提供 `electron44`；不以其他 Electron 主版本替代。

## 构建

在对应目录执行，使用普通用户：

```sh
cd packaging/aur/super-paste-git
makepkg -si
```

或：

```sh
cd packaging/aur/super-paste-bin
makepkg -si
```

## 准备发布

1. 将本次源码推送至上游仓库；`-git` 从该仓库构建，不使用维护者的本地修改。
2. 使用 `pnpm build:linux:portable` 生成 deb、pacman 和 AppImage，需要 Docker。`pnpm build:linux` 用于本机打包，会继承本机原生模块的系统库要求。
3. 在项目根目录执行 `pnpm aur:prepare`，或 `pnpm aur:prepare /path/to/super_paste_app_1.0.0_amd64.deb`。命令根据项目版本和实际 deb 更新校验和、同步桌面入口，并用 `makepkg --printsrcinfo` 生成两份 `.SRCINFO`，需要本机安装 `makepkg`。仅更新源码版时使用 `pnpm aur:prepare --variant git`；仅更新二进制版使用 `--variant bin`。
4. 将该 deb 原样上传至 GitHub Release，标签为 `v<package.json version>`，文件名保持 `super_paste_app_<version>_amd64.deb`。替换同名发布文件后必须重新生成校验和。
5. 两个 AUR 包分别维护独立仓库，上传各自目录中的 `PKGBUILD`、`.SRCINFO`，以及二进制版的 `super-paste`、`super-paste.desktop`。不要上传 deb、构建目录或完整项目源码。

`super-paste-bin` 复用上游安装包的压缩格式 `data.tar.xz`。源码版的桌面入口以 `packaging/arch` 为准；二进制版的桌面入口由准备命令同步。每次发布检查 `pkgrel`：仅修改 AUR 打包逻辑时递增，升级应用版本时重置为 1。

## Linux 二进制兼容性

发行包从 Ubuntu 22.04 容器构建，强制重编译 SQLite 和 X11 活动窗口模块，重新编译 KWin 模块，避免复用 Arch 宿主机上要求较新 glibc 的产物：

```sh
pnpm build:linux:portable
pnpm aur:prepare
```

此环境使用 Node 24、pnpm 11.4.0、Rust 1.88.0，构建上下文排除宿主机依赖和输出。打包钩子检查所有 ELF 文件，超过 GLIBC 2.35 或 GLIBCXX 3.4.30 即终止，阻止不兼容产物进入 Release。这对应 Ubuntu 22.04 的系统库基线，仍需要目标系统具备桌面、显示服务和图形驱动依赖；不提供 musl/Alpine 支持。本机产物可用 `pnpm check:linux-compat` 检查。

本地 OCR 的 ONNX 引擎和 Canvas 仍随应用分发，模型保持按需下载。Wayland 活动窗口检测支持 KDE Plasma 6；其他桌面按现有行为降级，不记录无法取得的来源窗口。

## GitHub Actions 发布

两个工作流都只配置 `workflow_dispatch`，提交代码、推送标签或创建 Release 均不会自动触发。

### Build and release

在 Actions 中选择该工作流，点击 **Run workflow**。默认构建 Linux x64，也可选 Windows x64、macOS x64 / arm64 或全部平台；可选标记为预发布。Windows 和 macOS 默认生成未签名产物。

版本取自 `package.json`，标签为 `v<version>`。发布前先升级版本；工作流拒绝覆盖已公开发布的同名 Release。全部选中平台成功后才上传并公开 Release，附带安装包和 `SHA256SUMS`。上传失败留下的本工作流草稿可以重跑恢复。

更新概览取当前提交历史中最近一次已发布 Release 到当前提交的全部提交，按功能、修复、性能、构建等分类，包含直接提交和合并提交；首次发布列出完整历史，并提供提交及版本差异链接。使用仓库自带的 `GITHUB_TOKEN`，无需另设发布 Token。

### Publish AUR

先在仓库 **Settings → Secrets and variables → Actions** 配置：

| 类型             | 名称                            | 内容                                         |
| ---------------- | ------------------------------- | -------------------------------------------- |
| Secret           | `AUR_SSH_PRIVATE_KEY`           | 已在 AUR 账户登记公钥的 SSH 私钥，无密码保护 |
| Variable（可选） | `AUR_GIT_NAME`、`AUR_GIT_EMAIL` | AUR 提交署名，默认使用项目维护者姓名和邮箱   |

私钥对应的账户须有目标 AUR 仓库的维护权限。上游源码与二进制下载须公开可访问，工作流会拒绝私有仓库。

无需配置 `AUR_SSH_KNOWN_HOSTS`：工作流自动获取 AUR 的 Ed25519 主机公钥并核对固定的[官方指纹](https://archlinux.org/news/aur-migration-new-ssh-hostkeys/)，保持严格主机校验。若 AUR 更换主机密钥，须先核对官方公告并更新工作流中的指纹。

手动运行工作流时选择 `git` 或 `bin`。`git` 从默认分支生成源码版元数据；`bin` 默认下载最新稳定 Release 的 deb，也可通过 `release_tag` 指定 `v<version>`。不重新构建二进制，不触发 GitHub Release 工作流。

工作流重新生成真实校验和和 `.SRCINFO`，只推送所选 AUR 仓库。应用版本变化时重置 `pkgrel=1`；同版本的打包变化自动递增，无变化跳过，版本倒退则失败。
