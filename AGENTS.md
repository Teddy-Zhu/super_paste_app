# Repository Guidelines

## 项目结构与模块组织
- `src/main` 负责 Electron 主进程、窗口生命周期与系统托盘逻辑；调整剪贴板监听时同步更新 `scripts/afterPack.js` 中的打包钩子以确保产物包含所需资源。
- `src/preload` 暴露受控桥接 API 给渲染层，请保持函数签名和安全校验，避免出现未授权的 Node 能力。
- `src/renderer/src` 是 Vue 3 前端，`components` 存放可复用 UI，`stores` 基于 Pinia 管理状态，`utils` 汇总剪贴板与 OCR 辅助函数；静态文件集中在 `assets`。
- `resources/ocr` 存放 ONNX 模型与字典，提交时确认未遗漏；构建输出位于 `out/` 与 `dist/`，请在 PR 中忽略这些目录。
- 路径别名定义在 `tsconfig.json`、`tsconfig.node.json` 与 `electron.vite.config.ts`，请保持 `@main`、`@renderer` 映射一致，新增模块时同步更新三处配置。

## 构建、测试与开发命令
- `pnpm install`：安装依赖并触发 `electron-builder` 的原生模块准备流程。
- `pnpm dev`：启动 electron-vite 开发模式，包含主进程、预加载与渲染进程的热重载，并开放端口 `9222` 便于调试。
- `pnpm start`：加载打包后的预览，用于复现用户环境或验证生产构建。
- `pnpm build`：先执行类型检查再生成生产包；根据目标平台使用 `pnpm build:win`、`pnpm build:mac` 或 `pnpm build:linux`。
- `pnpm lint`、`pnpm format`、`pnpm typecheck`：分别运行 ESLint、Prettier 与跨进程类型校验，提交前至少执行前两者。
- `pnpm run build:unpack`：生成未压缩目录结构，便于使用 `electron ./dist` 进行 smoke test 或离线调试。

## 编码风格与命名约定
- 使用 TypeScript 与 Vue 单文件组件，遵循 Prettier 默认的 2 空格缩进和自动换行规则，勿手动调整格式。
- 组件、布局文件命名采用 PascalCase（示例：`ClipboardCard.vue`），Pinia store 使用 `useXxxStore` 模式，普通工具函数保持 camelCase。
- 遵循 ESLint 规则（如 `vue/block-lang` 强制使用 `<script lang="ts">`），如需例外请在局部添加 `// eslint-disable-next-line` 并说明原因。
- 静态字符串存放在专门的常量文件或国际化配置中，避免散布在渲染模板。
- 优先使用 Composition API 与 `<script setup>`，确有需要使用 Options API 时在组件顶部添加注释说明权衡。

## 测试准则
- 项目暂未集成单元测试框架，最小门槛是通过 `pnpm lint` 与 `pnpm typecheck`；新增关键逻辑时请在 PR 描述写明手动验证步骤。
- 对剪贴板监听、OCR 识别、WebDAV 同步等高风险场景，建议编写临时脚本（例如参考 `test.js`）或提供复现命令并附带日志。
- 计划扩展自动化测试时，优先抽离纯函数到 `src/renderer/src/utils` 并使用 Vitest/Vitest DOM（需新增依赖）编写 `*.spec.ts` 文件，保持命名与目录结构一致。
- 记录调试信息时使用 `electron-log`，测试阶段可设置 `log.transports.file.level = 'debug'`，并在 PR 中附上关键日志片段。

## 提交与拉取请求指南
- Git 历史主体遵循类似 Conventional Commits 的格式（`feat: xxxx`、`fix: xxxx`）；请保持动词简洁并聚焦单一变更。
- 提交前确认未包含 `dist/`、`out/` 或大型模型文件的无关修改，必要时在 `.gitignore` 中补充规则。
- PR 描述需涵盖变更摘要、运行过的命令、潜在风险以及回滚策略；涉及 UI 变化请附截图或短视频。
- 若 PR 影响安装包体积或原生依赖，请在讨论区说明并抄送发布负责人，便于安排重新签名与验收。

## 配置与安全提示
- `electron-builder.yml` 管理多平台打包配置，调整渠道或签名证书时请在 PR 中注明目标环境并附验证结果。
- 敏感凭证通过系统钥匙串或环境变量注入，例如 `WEB_DAV_URL`、`WEB_DAV_TOKEN`，切勿硬编码到仓库或示例文件。
- 变更原生依赖（如 `better-sqlite3`、`onnxruntime-node`）后运行 `pnpm exec electron-rebuild --force` 并在目标平台安装包上手测，避免出现缺失符号或崩溃。
- 处理 OCR 模型更新时同步刷新 `resources/ocr` 下的校验信息，在发布记录中注明来源版本与检验方式，防止误用过期模型。
