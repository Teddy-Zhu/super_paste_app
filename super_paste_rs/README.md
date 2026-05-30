# SuperPaste

> 功能强大的跨平台剪贴板历史管理工具

SuperPaste 是一个使用 Tauri + Rust + Vue 3 构建的现代化剪贴板管理器，支持剪贴板历史记录、标签分类、OCR 文字识别和 WebDAV 云同步。

## ✨ 特性

- 📋 **剪贴板历史** - 自动记录所有复制内容（文本、图片、文件）
- 🔍 **全文搜索** - 快速查找历史记录
- 📌 **置顶与收藏** - 重要内容永久保存
- 🏷️ **标签分类** - 灵活的内容组织方式
- 🖼️ **OCR 识别** - 从图片中提取文字（计划中）
- ☁️ **WebDAV 同步** - 跨设备同步剪贴板（计划中）
- ⌨️ **全局快捷键** - 快速唤起 Overlay 窗口
- 🎨 **现代化 UI** - 简洁美观的用户界面
- 🌍 **多语言支持** - 中文/英文

## 🚀 快速开始

### 安装

从 [Releases](https://github.com/yourusername/super_paste_app/releases) 下载对应平台的安装包：

- **Windows**: `SuperPaste_x64-setup.exe` 或 `.msi`
- **macOS**: `SuperPaste_x64.dmg`
- **Linux**: `.AppImage` 或 `.deb`

### 使用

1. 启动应用后，它会在系统托盘中运行
2. 使用全局快捷键 `Ctrl+Shift+V` (Windows/Linux) 或 `Cmd+Shift+V` (macOS) 唤起 Overlay 窗口
3. 在 Overlay 中搜索并选择历史记录
4. 点击托盘图标打开主窗口进行更多操作

## 🛠️ 开发

### 环境要求

- Rust >= 1.70
- Node.js >= 18
- pnpm >= 8

详细依赖请参考 [BUILD.md](BUILD.md)

### 本地开发

```bash
# 克隆仓库
git clone https://github.com/yourusername/super_paste_app.git
cd super_paste_app/super_paste_rs

# 安装依赖
pnpm install

# 启动开发服务器
pnpm tauri dev
```

### 构建

```bash
# Linux/macOS
./build.sh

# Windows
build.bat
```

详细构建指南请参考 [BUILD.md](BUILD.md)

## 📁 项目结构

```
super_paste_rs/
├── src/                    # 前端代码 (Vue 3 + TypeScript)
│   ├── api/               # Tauri 命令封装
│   ├── stores/            # Pinia 状态管理
│   ├── pages/             # 页面组件
│   ├── components/        # 可复用组件
│   ├── composables/       # 组合式函数
│   ├── i18n/              # 国际化
│   └── router/            # 路由配置
├── src-tauri/             # 后端代码 (Rust)
│   ├── src/
│   │   ├── app/          # 应用初始化
│   │   ├── commands/     # Tauri 命令
│   │   ├── infrastructure/ # 基础设施层
│   │   ├── services/     # 业务逻辑层
│   │   ├── system/       # 系统集成
│   │   └── types/        # 类型定义
│   ├── migrations/       # 数据库迁移
│   └── benches/          # 性能测试
└── docs/                  # 文档
```

## 🗺️ 路线图

- [x] 阶段 0: 基础设施（错误处理、日志、配置）
- [x] 阶段 1: 数据库层（SQLite + 迁移）
- [x] 阶段 2: 剪贴板核心（监听、存储、去重）
- [x] 阶段 3: 高级功能（OCR/同步占位）
- [x] 阶段 4: 前端 UI/UX
- [x] 阶段 5: 打包配置
- [ ] 阶段 6: OCR 集成（oar-ocr）
- [ ] 阶段 7: WebDAV 同步实现
- [ ] 阶段 8: 性能优化与测试

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

## 📄 许可证

MIT License - 详见 [LICENSE](LICENSE)

## 🙏 致谢

- [Tauri](https://tauri.app/) - 跨平台应用框架
- [clipboard-rs](https://github.com/ChurchTao/clipboard-rs) - 剪贴板操作库
- [active-win-pos-rs](https://github.com/dimusic/active-win-pos-rs) - 活动窗口追踪
- [Vue 3](https://vuejs.org/) - 前端框架
