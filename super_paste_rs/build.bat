@echo off
REM SuperPaste 构建脚本 - Windows

echo === SuperPaste 构建脚本 ===
echo.

REM 检查依赖
where cargo >nul 2>nul
if %errorlevel% neq 0 (
    echo 错误: 需要安装 Rust
    exit /b 1
)

where pnpm >nul 2>nul
if %errorlevel% neq 0 (
    echo 错误: 需要安装 pnpm
    exit /b 1
)

REM 进入项目目录
cd /d "%~dp0"

echo 1. 安装前端依赖...
call pnpm install

echo.
echo 2. 构建前端...
call pnpm build

echo.
echo 3. 构建 Tauri 应用...
cd src-tauri
cargo tauri build

echo.
echo === 构建完成 ===
echo 产物位置: src-tauri\target\release\bundle\
