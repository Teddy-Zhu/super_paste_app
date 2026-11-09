# Repository Guidelines

## 项目范围与结构

- 当前仓库根目录是 Tauri 2 / Rust / Vue 项目，不再包含 Electron 宿主或 native 参考工程。
- `src/renderer/src/` 保存复用的 Vue UI，`src/shared/` 定义前后端契约；保持现有业务与交互语义。
- `src-tauri/src/` 实现窗口、平台接入、剪贴板、存储、OCR、同步与 MCP；数据库格式集中在 `crates/data-format/`。
- `tools/migrate-electron/` 是独立一次性迁移器，旧格式解析不进入主应用运行路径。
- `src-tauri/vendor/` 的来源、许可证与必要补丁需保留，不引入父目录源码或外部参考工程依赖。

## 开发与构建

- 在仓库根目录运行 `pnpm install`、`pnpm dev`。
- `pnpm build:renderer` 执行前端类型检查和生产构建；`pnpm build` 生成当前平台安装包。
- `pnpm build:pacman` 在 x86_64 Arch Linux 生成 DEB、Pacman 包与 AUR 元数据；`pnpm aur:prepare` 复用已有 DEB。
- 产品版本以 `package.json` 为唯一人工维护来源；`src-tauri/tauri.conf.json` 引用它，`pnpm version:sync` 同步主应用的 `Cargo.toml` 与 `Cargo.lock`。开发、打包和发布入口会自动同步；提交版本修改或创建标签前先运行该命令。
- ORT 由 `scripts/prepare-ort.mjs` 准备，模型按需下载；勿提交 `target/`、`dist/`、`node_modules/` 或下载的运行库。

## 实现与验证

- 沿用 Vue Composition API、TypeScript、Pinia 和现有样式体系；组件使用 PascalCase，store 使用 `useXxxStore`。
- Rust 保持模块职责清晰；后台持有权威配置，前端显式提交变更，命令与事件保持类型和窗口权限约束。
- 完成实现、接入和代码审阅后，再按改动范围集中执行已有 lint、类型检查或构建；不逐文件重复验证。
- 格式化仅处理本次改动文件；不搭建 UI 自动化，UI 由用户手动验收。
- 区分实现完成与验证通过，明确未运行的检查和未验收的平台；不因本机构建成功宣称跨平台业务通过。

## 数据与交付

- 应用标识为 `com.v2dawn.superpaste`；凭据仅存系统凭据库，不写入日志、诊断、普通配置或 WebDAV。
- WebDAV 仅同步非敏感设置，不增加历史数据库同步或 Electron 格式兼容。
- 保留 `.git` 历史与他人修改；未经明确要求，不暂存、提交、推送、发布或安装产物。
- PR 说明实际行为变化、必要验证与限制；平台问题附操作系统、桌面环境和复现步骤。
