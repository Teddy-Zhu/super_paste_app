# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SuperPaste 是一个基于 Electron 的高级剪贴板管理工具，支持剪贴板历史记录、标签管理、OCR 功能和数据同步。

## Development Commands

### Package Manager

使用 pnpm 作为包管理器：

```bash
pnpm install
```

### Development & Build Commands

```bash
# 开发模式（带 sourcemap 和远程调试）
pnpm dev

# 类型检查
pnpm typecheck            # 检查所有类型
pnpm typecheck:node      # 检查主进程类型
pnpm typecheck:web       # 检查渲染进程类型

# 代码格式化和检查
pnpm format              # 格式化代码
pnpm lint               # ESLint 检查

# 构建
pnpm build              # 构建应用
pnpm build:win          # 构建 Windows 版本
pnpm build:mac          # 构建 macOS 版本
pnpm build:linux        # 构建 Linux 版本
```

## Architecture Overview

### Core Structure

- **主进程** (src/main/): Electron 主进程，处理系统集成和核心业务逻辑
- **渲染进程** (src/renderer/): Vue 3 + TypeScript 前端界面
- **预载脚本** (src/preload/): 主进程与渲染进程通信桥梁

### Key Components

#### 主进程架构 (src/main/)

- **clipboard/**: 剪贴板管理核心 (clipboard-manager, clipboard-database)
- **sync/**: 数据同步服务，支持 WebDAV 等同步方式
- **ocr/**: 本地 OCR 功能，支持图片文字识别
- **ui/windows/**: 窗口管理 (主窗口和覆盖窗口)
- **ui/tray/**: 系统托盘管理
- **communication/**: IPC 通信管理
- **workers/**: 后台任务处理 (定时同步、历史清理)
- **system/**: 系统集成 (全局快捷键、电源管理)

#### 渲染进程架构 (src/renderer/)

- **pages/index/**: 主设置界面，包含各种设置组件
- **pages/overlay.vue**: 剪贴板历史覆盖窗口
- **components/**: 通用组件 (ClipboardCard, ImageDisplay 等)
- **composables/**: Vue 3 组合式函数，封装业务逻辑
- **stores/**: Pinia 状态管理

### Data Management

- **数据库**: 使用 better-sqlite3 存储剪贴板历史
- **配置管理**: 使用 electron-store 管理应用配置
- **同步服务**: 支持 WebDAV 同步数据库和配置文件

### Technology Stack

- **Electron**: 跨平台桌面应用框架
- **Vue 3**: 渲染进程 UI 框架，使用 Composition API
- **TypeScript**: 全项目类型安全
- **Pinia**: 状态管理
- **better-sqlite3**: 本地数据库
- **electron-vite**: 构建工具
- **ONNX Runtime**: OCR 模型推理

## Development Guidelines

### IPC Communication

主进程和渲染进程通过 IPC 通信，相关接口定义在：

- `src/main/communication/ipc-handlers.ts` - 主进程 IPC 处理器
- `src/renderer/src/api/ipc.ts` - 渲染进程 IPC 客户端

### State Management

使用 Pinia 管理渲染进程状态，主要 store：

- `stores/index.ts` - 主状态store
- `stores/sync.ts` - 同步相关状态

### Database Schema

剪贴板数据库表结构和操作在 `src/main/clipboard/clipboard-database.ts`

### Sync System

同步系统采用抽象设计模式，位于 `src/main/sync/`：

- 支持多种同步提供者（当前支持 WebDAV）
- 可配置同步方向（上传/下载/双向）
- 自动定时同步和手动同步

### OCR Integration

本地 OCR 功能使用 ONNX Runtime，相关代码在 `src/main/ocr/`

## Important Notes

### Build Dependencies

项目依赖原生模块，需要编译：

- `@paymoapp/active-window` - 活动窗口检测
- `better-sqlite3` - SQLite 数据库
- `canvas` - Canvas 绘制
- `onnxruntime-node` - OCR 模型推理

### Platform-Specific Features

- **macOS**: 需要权限配置 (entitlements.mac.plist)
- **Windows**: 支持 NSIS 安装包
- **Linux**: 支持 deb 和 pacman 包格式
