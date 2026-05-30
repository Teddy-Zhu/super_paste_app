#!/bin/bash
# SuperPaste 构建脚本 - Linux/macOS

set -e

echo "=== SuperPaste 构建脚本 ==="
echo ""

# 检查依赖
command -v cargo >/dev/null 2>&1 || { echo "错误: 需要安装 Rust"; exit 1; }
command -v pnpm >/dev/null 2>&1 || { echo "错误: 需要安装 pnpm"; exit 1; }

# 进入项目目录
cd "$(dirname "$0")"

echo "1. 安装前端依赖..."
pnpm install

echo ""
echo "2. 构建前端..."
pnpm build

echo ""
echo "3. 构建 Tauri 应用..."
cd src-tauri
cargo tauri build

echo ""
echo "=== 构建完成 ==="
echo "产物位置: src-tauri/target/release/bundle/"
