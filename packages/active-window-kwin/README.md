# @super-paste/active-window-kwin

通过 napi-rs 调用 kdotool 的 Rust library API，读取 KDE Plasma 6 的活动窗口。通过 KWin 的会话 D-Bus 接口工作，支持 Wayland 和 X11；不支持 GNOME、其他合成器或 Plasma 5，不需要安装 kdotool 命令行程序。

## 构建

需要 Linux、Rust/Cargo 1.88+、C 编译工具链。首次构建需要访问 crates.io 和 GitHub。D-Bus C 库从源码编译，不依赖系统的 libdbus 开发包。

```sh
pnpm install
pnpm run build:native
```

根项目的 `pnpm dev` 和 `pnpm build` 仅在目标架构的 `.node` 文件缺失时自动构建；文件存在时直接复用，不调用 Cargo。修改 Rust 源码或更新原生依赖后，单独执行 `pnpm run build:native` 重建。使用 Node-API 8，同一二进制可供 Node.js 和 Electron 使用，无须针对 Electron ABI 重编译。

支持 Linux glibc 的 x64 和 arm64；产物为包内的 `active_window_kwin.linux-{arch}-gnu.node`。默认构建本机架构，交叉构建时设置 `CARGO_BUILD_TARGET`，并自行配置对应的 Rust target、C 编译器和 linker。构建使用 Cargo.lock；原生产物与 target 目录不入库。

## Node.js API

```js
const activeWindow = require('@super-paste/active-window-kwin')

const support = activeWindow.getSupportStatus()
if (!support.supported) throw new Error(support.reason)

activeWindow.getActiveWindow().then((window) => {
  // 单次查询结果；读取失败时 Promise 会拒绝。
}, console.error)
const unsubscribe = activeWindow.subscribe(
  (window) => {
    // WindowInfo | null；没有活动窗口时为 null。
  },
  (error) => {
    // D-Bus、KWin 权限或原生模块加载失败。
  },
  500
)
unsubscribe()
```

`getSupportStatus()` 检查平台、架构和 KDE 会话版本，不代表已获 KWin 访问权限；实际权限和连接错误由查询的 Promise / 订阅错误回调返回。

`WindowInfo` 包含 `id`、`title`、`application`、`windowClass`、`pid`、`path`、`bounds`。`id` 为 KWin 窗口标识，`application` 使用窗口的 resourceClass；`path` 从 `/proc/<pid>/exe` 获取，进程退出或无权访问时为空。包不依赖 Electron，也不伪造缺失的路径。

查询在线程池运行；同一 JS 实例的并发查询复用一个 Promise，Rust 入口还会串行化同一进程的查询。订阅按“查询完成后再等待间隔”的方式轮询，默认 500 ms，仅窗口快照变化时回调。取消订阅会停止后续轮询和回调，已经运行的 D-Bus 请求仍由上游完成；上游请求可能等待数秒。错误之后的首次成功会重新发送快照。

应用的 `ActiveWindowService` 只在 Wayland 会话选择此后端，其他平台沿用原有后端；后端适配层处理图标、统一字段和生命周期，剪贴板业务不直接依赖此包。无法取得真实可执行路径时，不记录剪贴板来源应用。

上游固定为 [kdotool be03ce90](https://github.com/jinliu/kdotool/tree/be03ce90c09350898556436bac74ed35fe928617)，关闭其 CLI feature，仅链接 Rust library API。上游采用 Apache-2.0，许可随包保存在 `licenses/kdotool-LICENSE`。
