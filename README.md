# Super Paste

一款基于 Electron、Vue 3 和 TypeScript 的桌面剪贴板管理工具。保存复制历史，通过快捷面板搜索、分类和恢复内容，并提供图片 OCR 与可选的 WebDAV 同步。

[发布版本](https://github.com/Teddy-Zhu/super_paste_app/releases) · [问题反馈](https://github.com/Teddy-Zhu/super_paste_app/issues) · [参与贡献](#参与贡献)

## 功能

- **剪贴板历史**：记录文本、图片、文件等内容，支持详情预览；可设置记录数量与保存期限。
- **快捷面板**：通过自定义全局快捷键呼出，支持搜索、键盘导航和数字快捷选择；可从屏幕上、下、左、右四个方向出现。
- **分类管理**：创建带颜色和描述的标签，按标签或来源应用筛选记录。来源识别能力取决于操作系统与桌面环境。
- **图片 OCR**：可下载本地模型进行离线识别，或连接自定义远程 HTTP OCR 接口；支持保存图片时自动识别。
- **WebDAV 同步**：支持手动与定时同步、上传／下载／双向同步，以及仅同步配置。
- **桌面设置窗口**：支持调整窗口宽高，最小尺寸为 720 × 520，内容随可用空间换行和滚动。

## 安装与使用

可在 [Releases](https://github.com/Teddy-Zhu/super_paste_app/releases) 查看发布版本。如没有适合当前系统的安装包，可按下文从源码运行或打包。

1. 启动应用，进入设置窗口的「快捷键」页面，设置用于呼出粘贴面板的组合键。初始未配置全局快捷键。
2. 复制内容，应用会将其保存到本地历史记录。
3. 使用已配置的快捷键打开面板，搜索或切换分类，恢复需要的内容。
4. 按需配置保存期限、面板出现位置、标签、OCR 或同步服务。

面板中的常用键盘操作：

| 操作                        | 按键                               |
| --------------------------- | ---------------------------------- |
| 移动选择                    | 方向键                             |
| 选择首条／末条记录          | `Ctrl/⌘ + Home` / `Ctrl/⌘ + End`   |
| 恢复选中的内容              | `Enter`                            |
| 快速恢复当前列表前 9 条记录 | `Ctrl + 1…9`；macOS 使用 `⌘ + 1…9` |
| 退出当前搜索或关闭面板      | `Esc`                              |

### OCR

在「OCR 设置」中选择本地模型或远程接口，修改后点击保存配置。

- **本地识别**：首次使用需要下载所选模型；下载完成后可以离线识别。
- **远程识别**：配置接口地址、JSON 或 multipart 表单格式、图片字段、额外参数及返回结果路径。待识别图片会发送到所配置的服务。
- **自动识别**：启用后，保存图片到历史记录时自动执行 OCR。

### WebDAV 同步

在「同步设置」中填写服务器地址、用户名、密码与远程目录，可先测试连接，再保存配置。支持手动同步、定时同步和仅同步配置；下载数据后按应用提示重启以生效。

同步以数据库和配置文件为单位。双向同步根据文件修改时间选择上传或下载，不会逐条合并两台设备的历史记录；多设备同时修改时，请留意文件覆盖。

## 平台说明

项目提供 Windows、macOS 和 Linux 的打包配置。平台相关能力受系统权限、桌面环境和原生依赖支持范围影响。

| 平台或环境      | 说明                                                                                           |
| --------------- | ---------------------------------------------------------------------------------------------- |
| Windows / macOS | 提供对应打包命令；macOS 来源应用识别可能需要屏幕录制权限。                                     |
| Linux X11       | 使用原生活动窗口后端识别来源应用。                                                             |
| Linux Wayland   | 来源应用识别使用 KWin 后端，当前支持 KDE Plasma 6；该后端不支持 GNOME、其他合成器或 Plasma 5。 |

Wayland 下的全局快捷键可能需要桌面门户授权，可在「快捷键」页面查看状态并重试。如果桌面环境无法注册，可在系统快捷键设置中绑定应用可执行文件并附加 `--toggle-overlay` 参数；页面会显示当前可执行文件的完整命令。

KWin 后端的构建要求与支持范围见 [原生模块说明](packages/active-window-kwin/README.md)。

## 从源码运行

### 环境要求

- Node.js **22.14.0 或更高版本**。
- pnpm **11.4.0**，与 `package.json` 的 `packageManager` 保持一致。
- 原生模块需要与当前操作系统和架构匹配；没有可用预编译产物时，需要相应的 C/C++ 编译环境。
- Linux 首次构建缺失的 KWin 原生模块时，还需要 **Rust / Cargo 1.88+** 与 C 编译工具链，并能访问 crates.io 和 GitHub。

在仓库根目录执行：

```sh
git clone https://github.com/Teddy-Zhu/super_paste_app.git
cd super_paste_app
pnpm install
pnpm dev
```

安装依赖会执行 Electron 原生依赖准备。开发模式支持热更新，并开放 `9222` 调试端口。Linux 上，`pnpm dev` 和 `pnpm build` 会在缺失目标原生产物时构建 KWin 模块；修改该模块的 Rust 源码后，请显式运行 `pnpm run build:native` 重建。

### 常用命令

| 命令                    | 用途                                     |
| ----------------------- | ---------------------------------------- |
| `pnpm dev`              | 启动 Electron 开发模式                   |
| `pnpm lint`             | 运行 ESLint                              |
| `pnpm typecheck`        | 检查主进程、预加载与渲染进程类型         |
| `pnpm format`           | 格式化项目文件，会写入文件               |
| `pnpm build`            | 准备原生模块、检查类型并生成应用构建产物 |
| `pnpm start`            | 预览已有构建产物，需先运行 `pnpm build`  |
| `pnpm run build:native` | 重建 KWin 原生模块                       |
| `pnpm run build:unpack` | 构建并生成未封装的应用目录               |

### 打包

建议在对应目标系统上准备原生依赖并打包：

```sh
# Windows
pnpm build:win

# macOS
pnpm build:mac

# Linux
pnpm build:linux
```

应用构建产物位于 `out/`，安装包与打包目录位于 `dist/`。当前 Linux 配置默认生成 x64 的 `.deb` 和 pacman 软件包。目标、图标、签名及发布设置见 [electron-builder.yml](electron-builder.yml)。

## 数据与隐私

历史记录与配置保存在 Electron 的用户数据目录，即 `app.getPath('userData')`：

| 文件或目录             | 内容                           |
| ---------------------- | ------------------------------ |
| `clipboard-history.db` | 剪贴板历史、标签和来源应用信息 |
| `config.json`          | 应用设置与同步配置             |
| `ocr-models/`          | 下载的本地 OCR 模型            |

本地 OCR 的模型下载需要联网，识别本身在本机执行。远程 OCR 会向配置的接口提交图片；启用 WebDAV 同步后，会向配置的服务器传输配置文件，以及同步范围内的历史数据库。剪贴板内容与配置可能包含敏感信息，请妥善保护本地数据与同步账户。当前历史数据库未启用数据库加密。

## 项目结构

```text
src/
  main/          # Electron 主进程、窗口、剪贴板、数据库、OCR 与同步
  preload/       # 渲染进程可调用的受控桥接 API
  renderer/src/  # Vue 界面、组件、状态与交互
  shared/        # 跨进程共享类型和常量
packages/
  active-window-kwin/  # KDE Plasma 6 活动窗口原生模块
resources/       # 应用资源
scripts/         # 打包钩子等脚本
```

## 参与贡献

欢迎通过 [Issues](https://github.com/Teddy-Zhu/super_paste_app/issues) 报告问题或提出建议，通过 Pull Request 提交改进。

- 报告问题时，请提供应用版本、操作系统、桌面环境、复现步骤和预期行为；UI 问题可附截图。
- 修改前先阅读 [仓库开发规范](AGENTS.md)，尽量使每个改动围绕一个明确问题。
- 提交前运行 `pnpm lint`、`pnpm typecheck`，并格式化修改过的文件。
- PR 中说明改动目的、验证结果及已知限制；涉及界面或平台能力时，附上实际运行环境和验证说明。
- 请勿提交构建产物、下载的 OCR 模型、个人剪贴板数据或同步凭据。

## 许可证

当前仓库尚未提供项目级许可证文件。第三方依赖与 OCR 模型的许可请查阅各自项目；KWin 模块使用的 kdotool 许可随模块保存在 [kdotool-LICENSE](packages/active-window-kwin/licenses/kdotool-LICENSE)。
