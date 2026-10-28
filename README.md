# tauri-plugin-proxmark3

把 [Proxmark3](https://github.com/RfidResearchGroup/proxmark3) 客户端（`pm3`）编译成静态库，并以
[Tauri 2](https://v2.tauri.app/develop/plugins/) 插件的形式暴露给前端 / Rust 后端调用。

- 通过 **git submodule** 固定 Proxmark3 版本（`proxmark3/`）
- 复用上游已有的库化实现 `client/experimental_lib`（`libpm3`，导出 `pm3.h`），不重复造轮子
- 额外提供一个 C bridge，解决上游 `pm3_open()` 打不开端口时直接 `exit()` 的问题，并补全输出抓取
- 一次编译，桌面 / Android / iOS 通用：全部静态链接，无运行时 `.so` / `.dll` 依赖

## 支持矩阵

| 平台 | 状态 | 传输方式 |
| --- | --- | --- |
| Android（arm64-v8a / armeabi-v7a / x86_64） | ✅ CI 构建 | 离线模式、`tcp:` / `udp:` 网络桥 |
| iOS（arm64 真机 / arm64 模拟器） | ✅ CI 构建 | 离线模式、`tcp:` / `udp:` 网络桥 |
| Linux x86_64 | ✅ CI 构建 + 测试 | 串口 / `tcp:` / `udp:` / `bt:`（Linux 构建） |
| macOS arm64 | ✅ CI 构建 + 测试 | 串口 / `tcp:` / `udp:` |
| Windows x86_64（GNU ABI / MinGW） | ✅ CI 构建 + 测试 | 串口（`COM3`）/ `tcp:` / `udp:` |

> Windows 只能使用 `x86_64-pc-windows-gnu`。上游客户端是 GNU 风格 C 代码（`__attribute__((packed))`、
> POSIX 头文件、pthread），无法用 MSVC / clang-cl 编译，因此本 crate 在 `*-msvc` 目标上会给出明确报错并中止。

## 工作原理

```
┌────────────────────────────┐
│ 前端 (JS/TS)                │  invoke('plugin:proxmark3|execute', { command })
└─────────────┬──────────────┘
              │ Tauri IPC（权限: proxmark3:default）
┌─────────────▼──────────────┐
│ Rust 插件（本 crate）       │  全局 Mutex + spawn_blocking，串行化客户端调用
└─────────────┬──────────────┘
              │ C ABI（shim/pm3_bridge.c）
┌─────────────▼──────────────┐
│ libpm3（static）            │  proxmark3/client/experimental_lib 的 CMake 目标
│ + mbedtls / lua / jansson … │  pm3rrg_rdv4_static 及其依赖
└────────────────────────────┘
```

`build.rs` 做的事情：

1. 用 CMake 构建 `proxmark3/client/experimental_lib` 的 `pm3rrg_rdv4_static` 目标；
2. 用仓库内自带的 `vendor/bzip2`、`vendor/lz4` 满足上游对这两个库的强制依赖（Android/iOS/Windows 上通常没有系统包）；
3. 读取 CMake 生成的静态库清单与 include 目录，翻译成 `cargo:rustc-link-*`；
4. 用 `cc` 编译 `shim/pm3_bridge.c`，把 `pm3_open` / `pm3_console` 封装成不会 `exit()`、
   能完整抓取 stdout（包括 `help` 这类绕过上游抓取缓冲的输出）的 API。

## 安装

```toml
# 应用 crate 的 Cargo.toml
[dependencies]
tauri-plugin-proxmark3 = { git = "https://github.com/xianglin1998/tauri-plugin-proxmark3", tag = "v0.1.0" }
# 或者从 GitHub Release 下载 .crate 后本地 path 依赖
```

```rust
// src/lib.rs / src/main.rs
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_proxmark3::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

在 `src-tauri/capabilities/default.json` 中授权：

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["main"],
  "permissions": ["core:default", "proxmark3:default"]
}
```

### 前端

```ts
import { connect, execute, status, disconnect } from 'tauri-plugin-proxmark3-api';

// 离线模式（无需硬件），或自动探测第一个设备
await connect();

// 指定传输：
//   串口      "/dev/ttyACM0" / "COM3"
//   网络桥    "tcp:192.168.1.20:18888" / "udp:192.168.1.20:18888"
//   蓝牙      "bt:AA:BB:CC:DD:EE:FF"（仅 Linux + BlueZ 构建）
await connect('tcp:192.168.1.20:18888');

const { status: code, output } = await execute('hw version');
console.log(code, output);
```

`guest-js/` 里是这组绑定的源码，可直接复制进项目，或自行打包成 npm 包。

## API

| 命令 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `connect` | `port?: string \| null` | `SessionInfo` | 打开会话；`null` 表示离线/自动探测 |
| `disconnect` | – | `SessionInfo` | 关闭会话 |
| `execute` | `command: string` | `CommandOutput` | 执行一条客户端命令；未连接时自动以离线模式打开 |
| `status` | – | `SessionInfo` | 当前会话状态 |
| `version` | – | `string` | 编译进库的客户端版本 |

```ts
interface SessionInfo { open: boolean; offline: boolean; port: string; version: string }
interface CommandOutput { status: number; output: string; durationMs: number }
```

Rust 侧也可以不经 Tauri 命令层直接调用：

```rust
use tauri_plugin_proxmark3::client;

client::connect(None)?;
let out = client::execute("hf search")?;
println!("{}", out.output);
```

## 构建要求

| 依赖 | 说明 |
| --- | --- |
| Rust stable | ≥ 1.77.2 |
| CMake ≥ 3.16 | 用于构建上游 `experimental_lib` |
| Ninja | 可选但推荐（`build.rs` 检测到就会用），否则用平台默认 generator |
| C/C++ 编译器 | Linux/macOS: gcc/clang；Android: NDK clang；iOS: Apple clang；Windows: MSYS2 UCRT64 gcc |
| zlib | 找到就会启用上游的 zlib 支持（Linux: `zlib1g-dev`） |

Windows（MSYS2 UCRT64）：

```powershell
# MSYS2: pacman -S mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-cmake mingw-w64-ucrt-x86_64-ninja
rustup target add x86_64-pc-windows-gnu
$env:PM3X_MINGW_BIN = "C:\msys64\ucrt64\bin"   # 如不在默认位置
cargo build --target x86_64-pc-windows-gnu
```

Android（已装 NDK，`cargo install cargo-ndk`）：

```bash
export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/27.0.12077973
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o jniLibs build --release
```

iOS（macOS + Xcode）：

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios
```

### 环境变量

| 变量 | 作用 |
| --- | --- |
| `CMAKE` | 指定 cmake 可执行文件 |
| `PM3X_MINGW_BIN` | Windows 下 gcc/ar 所在目录（默认自动探测 `C:\msys64\ucrt64\bin` 等） |
| `PM3X_CMAKE_GENERATOR` | 强制 CMake generator |
| `PM3X_CMAKE_ARGS` | 追加给 CMake configure 的参数 |
| `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT` / `ANDROID_NDK` / `ANDROID_NDK_LATEST_HOME` | NDK 位置 |
| `PM3X_ANDROID_API` | Android API level，默认 24 |
| `PM3X_IOS_DEPLOYMENT_TARGET` | iOS 部署目标，默认 13.0 |

## 本地开发

```bash
git submodule update --init --recursive   # 拉取 proxmark3（约 130 MB）
cargo test --target x86_64-pc-windows-gnu # 离线模式端到端测试，无需硬件
```

`tests/offline.rs` 会在没有 Proxmark3 硬件的情况下打开离线会话，执行 `hw version`、`data -h`
和一条不存在的命令，验证「编译 → 链接 → 命令解析 → 输出抓取」整条链路。

## CI / 发布

- `.github/workflows/ci.yml`：Linux / macOS / Windows(GNU) 构建 + 测试；Android 三个 ABI；iOS 模拟器与真机；`cargo package` 校验
- `.github/workflows/release.yml`：推送 `v*` tag 时，把包含 proxmark3 源码的 crate 打包成 `.crate`
  并自动发布到 GitHub Release（可选：配置 `CARGO_REGISTRY_TOKEN` 后同时发布到 crates.io）

```bash
git tag v0.1.0
git push origin v0.1.0
```

> `cargo package` 默认不会打包 git submodule 的内容，所以发布流程会把源码树复制到临时目录再打包，
> 这样发布出去的 `.crate` 是自包含、可离线构建的。

## 已知限制

- **许可证**：Proxmark3 客户端是 GPLv3。本插件静态链接了它，分发时整体需遵循 GPLv3；
  iOS App Store 的条款与 GPLv3 存在冲突，上架前请自行评估（可考虑仅内部/企业分发）。
- **Windows**：仅支持 GNU ABI；`*-msvc` 目标会在构建期报错并给出说明。
- **蓝牙**：当前 CMake 选项 `SKIPBT=1`（BlueZ 是 Linux 专有依赖），需要时可在 `cmake/CMakeLists.txt` 打开。
- **Android 串口**：普通 App 无权直接打开 `/dev/ttyACM*`，推荐用 `tcp:` 网络桥或离线模式。
  iOS 没有 USB Host，同理。
- 上游 `pm3_console()` 的抓取缓冲会跳过 `help` 输出，本插件改为重定向 stdout，因此一次 `execute`
  期间进程的 stdout 会被临时重定向（插件内部已串行化）。

## 目录结构

```
proxmark3/          git submodule（上游仓库）
cmake/              包装上游 experimental_lib 的 CMake 工程
shim/               C bridge（pm3_bridge.c/.h）
src/                Rust 插件：Tauri 命令 + FFI 封装
guest-js/           TypeScript 绑定
vendor/             自带 bzip2 / lz4 源码
tests/              离线模式端到端测试
_temp/              本地临时环境（已 gitignore）
```
