# src/main 后端代码优化计划

## 代码评估
- `src/main/communication/ipc-manager.ts` 的 `registerClipboardHandlers` 体量过大且重复 `ipcMain.handle` 的 try/catch 模板，导致新增能力时容易出现漏改；其中 `clipboard-generate-qrcode` 通过批量查询再筛选单条记录，效率低且多余。
- `src/main/services/clipboard-history-service.ts` 只暴露批量查询方法，主进程若要获取单条历史记录只能走额外的列表查找，缺少清晰的单项读取接口。
- `src/main/clipboard/clipboard-manager.ts` 中大量方法都只是调用 `this.call` 并包裹相似的异常处理与日志，重复代码多且容易出现行为不一致。
- `src/main/index.ts` (SuperPasteApp) 在处理窗口聚焦逻辑时散落着多处判定与日志，可抽提统一的“确保主窗口可见”逻辑以减少分支重复，同时后台初始化步骤完全串行，存在可以并行的轻量操作。

## 优化计划
1. `services/clipboard-history-service.ts`：补充根据 `contentId` 返回 `ClipboardItem` 的便捷方法，复用现有转换逻辑，供主进程直接获取单条记录。
2. `communication/ipc-manager.ts`：引入轻量工具函数统一广播与错误包装，将 `registerClipboardHandlers` 拆分为更易维护的片段，同时改用步骤 1 的单条查询优化二维码生成流程。
3. `clipboard/clipboard-manager.ts`：提取内部 `safeCall` 辅助函数，替换重复的 `try/catch` 模板，保持日志与默认值一致。
4. `index.ts`：收敛窗口聚焦逻辑、对部分后台初始化任务使用 Promise 并发（在不改变依赖关系的前提下），让启动流程更清晰。

---

## 第二轮代码评估
- `communication/ipc-manager.ts` 中的标签与历史更新通知经常同时调用 `notifyClipboardHistoryUpdated` 与 `notifyTagsUpdated`，可以合并为统一的状态通知工具，减少重复调用。
- 同文件在 `registerClipboardControlHandlers` 内部存在多处“直接透传到 Worker”或“固定响应模板”的 handler，可通过配置化注册降低样板代码。
- `services/clipboard-tag-service.ts` 中每个方法都使用相似的 try/catch 日志模板，维护负担较大，适合抽取通用的执行辅助函数。

## 第二轮优化计划
5. 为 `ipc-manager.ts` 添加统一的 `notifyStateChange` 方法，使同时更新历史与标签的逻辑更简洁可读。
6. 重构 `ipc-manager.ts` 的剪贴板控制 handler，通过表驱动注册模式复用相同的透传逻辑。
7. 在 `clipboard-tag-service.ts` 中引入通用的 `execute` 辅助函数，减少重复的错误处理代码。

---

## 第三轮代码评估
- `clipboard-manager.ts` 新增的 `safeCall` 目前以函数形式接收回退值，导致调用处频繁传入 `() => fallback` 并引发 Prettier 警告，可改为直接传递常量值以简化写法。
- `clipboard-tag-service.ts` 的 `execute` 与 `safeCall` 类似，同样可以接受直接的默认值，进一步降低样板代码。

## 第三轮优化计划
8. 调整 `clipboard-manager.ts` 中的 `safeCall` 签名，允许传入直接的回退结果，同时更新相关调用以消除多余的箭头函数。
9. 同步优化 `clipboard-tag-service.ts` 的 `execute` 辅助函数，统一使用常量回退值，保持 API 一致性。
