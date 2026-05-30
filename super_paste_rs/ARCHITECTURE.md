# 前端架构重构说明

## 问题

原实现将设置页面和剪贴板历史弹窗混在一起，使用 Vue Router 切换，不符合原项目的双窗口架构。

## 解决方案

### 1. 双窗口架构

**主窗口 (main)** - 设置页面
- URL: `index.html`
- 尺寸: 1000x700
- 特性: 可调整大小、有装饰、居中显示
- 内容: 设置页面（通用、同步、OCR、快捷键等）

**Overlay 窗口 (overlay)** - 剪贴板历史弹窗
- URL: `overlay.html`
- 尺寸: 380x520
- 特性: 置顶、无装饰、透明背景、不可调整大小
- 内容: 剪贴板历史列表 + 搜索

### 2. 文件结构

```
super_paste_rs/
├── index.html              # 主窗口入口
├── overlay.html            # Overlay 窗口入口
├── src/
│   ├── main.ts            # 主窗口入口脚本
│   ├── overlay.ts         # Overlay 窗口入口脚本
│   ├── App.vue            # 主窗口根组件（设置页面）
│   ├── OverlayApp.vue     # Overlay 窗口根组件（剪贴板历史）
│   ├── pages/
│   │   ├── SettingsPage.vue  # 设置页面组件
│   │   └── HomePage.vue      # （已废弃，不再使用）
│   ├── api/               # Tauri 命令封装
│   ├── stores/            # Pinia 状态管理
│   ├── composables/       # 可复用逻辑
│   └── i18n/              # 国际化
```

### 3. Vite 配置

```typescript
// vite.config.ts
build: {
  rollupOptions: {
    input: {
      main: path.resolve(__dirname, 'index.html'),
      overlay: path.resolve(__dirname, 'overlay.html'),
    },
  },
}
```

### 4. Tauri 配置

```json
// tauri.conf.json
"windows": [
  {
    "label": "main",
    "url": "index.html",
    "width": 1000,
    "height": 700,
    "decorations": true
  },
  {
    "label": "overlay",
    "url": "overlay.html",
    "width": 380,
    "height": 520,
    "alwaysOnTop": true,
    "transparent": true,
    "decorations": false
  }
]
```

### 5. 主要变更

#### 移除 Vue Router
- 主窗口直接渲染 `SettingsPage.vue`
- Overlay 窗口直接渲染 `OverlayApp.vue`
- 不再需要路由切换

#### 独立的入口文件
- `main.ts`: 主窗口入口（Pinia + i18n）
- `overlay.ts`: Overlay 窗口入口（Pinia + i18n）

#### 窗口间通信
- Overlay 中点击"设置"按钮 → 隐藏 Overlay + 显示主窗口
- 主窗口通过 Tauri 命令控制 Overlay 显示/隐藏

### 6. 构建产物

```
dist/
├── index.html              # 主窗口
├── overlay.html            # Overlay 窗口
├── assets/
│   ├── main-*.js          # 主窗口脚本
│   ├── main-*.css         # 主窗口样式
│   ├── overlay-*.js       # Overlay 窗口脚本
│   ├── overlay-*.css      # Overlay 窗口样式
│   └── index-*.js         # 共享依赖（Vue、Pinia 等）
```

### 7. 开发与构建

```bash
# 开发模式
pnpm tauri dev

# 生产构建
pnpm build
pnpm tauri build
```

### 8. 验证结果

✅ TypeScript 类型检查通过  
✅ Vite 多入口构建成功  
✅ Tauri 开发服务器启动成功  
✅ 两个独立的 HTML 页面生成

---

## 与原项目对比

| 特性 | 原项目 (Electron) | 当前实现 (Tauri) |
|------|------------------|------------------|
| 主窗口 | `pages/index` | `index.html` (SettingsPage) |
| Overlay | `pages/overlay` | `overlay.html` (OverlayApp) |
| 路由 | Vue Router | 无（独立窗口） |
| 窗口管理 | Electron BrowserWindow | Tauri Window API |
| 构建工具 | Vite | Vite (多入口) |

---

## 后续优化

1. **样式优化**: 参考原项目的 UI 设计，完善 Overlay 的动画效果
2. **功能完善**: 添加快捷键选择、标签过滤等功能
3. **性能优化**: 虚拟滚动、懒加载
4. **测试**: 手动测试两个窗口的交互
