# tauri-plugin-proxmark3

[English](README.md) | **简体中文**

把 [Proxmark3](https://github.com/RfidResearchGroup/proxmark3) 客户端（`pm3`）编译成静态库，并以
[Tauri 2](https://v2.tauri.app/develop/plugins/) 插件的形式暴露给前端 / Rust 后端调用。

最新构建产物（`.crate`、固件、客户端资源）：
[GitHub Releases](https://github.com/xianglin1998/tauri-plugin-proxmark3/releases)

- 通过 **git submodule** 固定 Proxmark3 版本（`proxmark3/`）
- **直接使用上游** `client/experimental_lib`（导出 `pm3.h` 的 `libpm3`）作为 CMake 工程；
  submodule 不会被留下改动：`patch/proxmark3/` 里的少量兼容文件只在 CMake 构建期间替换进去，
  构建结束自动还原；本仓库也不 vendor 任何第三方库
- bzip2 / lz4 由上游 CMake 自带的 `EMBED_BZIP2` / `EMBED_LZ4`（ExternalProject）在构建时下载编译
- iOS 交叉编译使用 [leetal/ios-cmake](https://github.com/leetal/ios-cmake)
- 只额外提供一个 C bridge（`shim/`），解决上游 `pm3_open()` 失败时 `exit()` 的问题，并支持**实时流式输出**
- 一次编译，桌面 / Android / iOS 通用：全部静态链接，无运行时 `.so` / `.dll` 依赖
- 自带 `examples/minimal-app`：一个依赖本插件的最小 Tauri 应用，CI 会在五个平台分别编译它，
  只有全部通过才发布 release

## 支持矩阵

| 平台 | 状态 | 传输方式 |
| --- | --- | --- |
| Android（arm64-v8a / armeabi-v7a / x86_64） | ✅ CI 构建 | 离线模式、`tcp:` / `udp:` 网络桥 |
| iOS（arm64 真机 / arm64 模拟器） | ✅ CI 构建（ios-cmake） | 离线模式、`tcp:` / `udp:` 网络桥 |
| Linux x86_64 | ✅ CI 构建 + 测试 | 串口 / `tcp:` / `udp:` / `bt:`（Linux 构建） |
| macOS arm64 | ✅ CI 构建 + 测试 | 串口 / `tcp:` / `udp:` |
| Windows x86_64（GNU ABI / MinGW） | ✅ CI 构建 + 测试 | 串口（`COM3`）/ `tcp:` / `udp:` |

> Windows 只能使用 `x86_64-pc-windows-gnu`。上游客户端是 GNU 风格 C 代码（`__attribute__((packed))`、
> POSIX 头文件、pthread），无法用 MSVC / clang-cl 编译，因此本 crate 在 `*-msvc` 目标上会给出明确报错并中止。

## 工作原理

```
┌────────────────────────────┐
│ 前端 (JS/TS)                │  invoke('plugin:proxmark3|execute')
└─────────────┬──────────────┘
              │ Tauri IPC（权限: proxmark3:default）
┌─────────────▼──────────────┐
│ Rust 插件（本 crate）       │  全局 Mutex + spawn_blocking；流式输出走事件
└─────────────┬──────────────┘
              │ C ABI（shim/pm3_bridge.c）
┌─────────────▼──────────────┐
│ libpm3（static）            │  proxmark3/client/experimental_lib: pm3rrg_rdv4_static
│ + mbedtls / lua / jansson … │  以及上游 deps 里的静态库
└────────────────────────────┘
```

`build.rs` 做的事情：

1. 先把 `patch/proxmark3/` 里的兼容文件覆盖到 submodule（构建脚本退出时自动还原），再对
   `proxmark3/client/experimental_lib` 执行 `cmake -S ... -B ...`，打开
   `SKIPBT/SKIPPYTHON/SKIPREADLINE/SKIPLINENOISE/SKIPGD`，bzip2 / lz4 走 `EMBED_*`；
2. 通过 `-DCMAKE_PROJECT_INCLUDE=cmake/pm3x_configure.cmake` 注入少量**编译器选项**：
   - Android x86/x86_64 的 `cipher_bs_avx*.c` 需要 `-mavx2` / `-mavx512f`（上游只给非 Android 加了 target pragma）；
   - iOS 没有 `system()`，用 `-include shim/ios_system.h` 把它替换成一个内联 stub，而不是改源码；
   - 把上游的 `-Werror` 降级为警告（消费方编译器版本不受我们控制）；

   这个文件不创建 project、不创建 target，也不改动上游的构建文件；
3. `cmake --build ... --target pm3rrg_rdv4_static`；
4. 按依赖顺序把生成的 `.a` 交给 `cargo:rustc-link-lib`，系统库（zlib / pthread / ws2_32 / 系统框架）按平台补齐；
5. 用 `cc` 编译 `shim/pm3_bridge.c`。

### bzip2 / lz4 从哪里来

- Proxmark3 仓库本身**没有**内置 bzip2 / lz4（`client/deps` 里没有这两个目录）；
- 上游 `client/experimental_lib/CMakeLists.txt` 提供了 `EMBED_BZIP2` / `EMBED_LZ4`：
  通过 ExternalProject 从 `android.googlesource.com` 拉取源码并用 `make` 构建；
- 这条路径依赖 POSIX shell（命令里用了 `mkdir -p`、管道等），因此在 **Linux / macOS / Android / iOS** 上使用；
- 原生 Windows 的 CMake 生成器用 `cmd.exe` 执行命令，`mkdir -p` 不成立，所以 Windows 上改用 MSYS2 的
  `mingw-w64-ucrt-x86_64-bzip2` / `-lz4` 包（见“构建要求”）。

## 安装

```toml
# 应用 crate 的 Cargo.toml
[dependencies]
tauri-plugin-proxmark3 = { git = "https://github.com/xianglin1998/tauri-plugin-proxmark3", tag = "v0.2.5" }
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
import { connect, execute, executeStream, status, disconnect } from 'tauri-plugin-proxmark3-api';

// 离线模式（无需硬件），或自动探测第一个设备
await connect();

// 指定传输：
//   串口      "/dev/ttyACM0" / "COM3"
//   网络桥    "tcp:192.168.1.20:18888" / "udp:192.168.1.20:18888"
//   蓝牙      "bt:AA:BB:CC:DD:EE:FF"（仅 Linux + BlueZ 构建）
await connect('tcp:192.168.1.20:18888');

// 一次性拿到完整输出
const { status: code, output } = await execute('hw version');

// 实时输出：命令执行过程中就能拿到每一段输出
await executeStream('hf search', (chunk) => {
  console.log('streamed:', chunk);
});
```

流式输出通过 Tauri 事件 `proxmark3://output` 传递（payload: `{ chunk: string }`），
`executeStream()` 会自动订阅 / 退订。

## API

| 命令 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `connect` | `port?: string \| null` | `SessionInfo` | 打开会话；`null` 表示离线/自动探测 |
| `disconnect` | – | `SessionInfo` | 关闭会话 |
| `execute` | `command: string` | `CommandOutput` | 执行一条客户端命令；未连接时自动以离线模式打开 |
| `execute_stream` | `command: string` | `CommandOutput` | 执行命令，同时通过 `proxmark3://output` 事件推送输出 |
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

// 流式
client::execute_stream("hf search", |chunk| print!("{chunk}"))?;
```

## 构建要求

| 依赖 | 说明 |
| --- | --- |
| Rust stable | ≥ 1.77.2 |
| CMake ≥ 3.19 | 上游要求 3.16；本插件用 `cmake_language(DEFER)` 注入编译选项，需要 3.19 |
| Ninja | 可选但推荐（`build.rs` 检测到就会用），否则用平台默认 generator |
| C/C++ 编译器 | Linux/macOS: gcc/clang；Android: NDK clang；iOS: Apple clang；Windows: MSYS2 UCRT64 gcc |
| git / make / tar | bzip2、lz4 的 ExternalProject 下载与构建（Windows 不需要） |
| zlib | 找到就会启用上游的 zlib 支持（Linux: `zlib1g-dev`） |
| Linux 桌面 | Tauri 本身依赖 GTK/WebKitGTK：`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` |

Windows（MSYS2 UCRT64）：

```powershell
# MSYS2 里安装：
#   pacman -S mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-cmake \
#             mingw-w64-ucrt-x86_64-ninja mingw-w64-ucrt-x86_64-zlib \
#             mingw-w64-ucrt-x86_64-bzip2 mingw-w64-ucrt-x86_64-lz4
rustup target add x86_64-pc-windows-gnu
$env:PM3X_MINGW_BIN = "C:\msys64\ucrt64\bin"   # 如不在默认位置
cargo build --target x86_64-pc-windows-gnu
```

Android（已装 NDK，`cargo install cargo-ndk`）：

```bash
export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/27.0.12077973
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o jniLibs build --release
```

iOS（macOS + Xcode + [ios-cmake](https://github.com/leetal/ios-cmake)）：

```bash
git clone --depth 1 https://github.com/leetal/ios-cmake.git _temp/ios-cmake
export PM3X_IOS_TOOLCHAIN="$PWD/_temp/ios-cmake/ios.toolchain.cmake"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios
```

> 未设置 `PM3X_IOS_TOOLCHAIN` 时，`build.rs` 会自动把 ios-cmake 克隆到系统临时目录（需要能访问 GitHub）。

### 环境变量

| 变量 | 作用 |
| --- | --- |
| `CMAKE` | 指定 cmake 可执行文件 |
| `PM3X_MINGW_BIN` | Windows 下 gcc/ar 所在目录（默认自动探测 `C:\msys64\ucrt64\bin` 等） |
| `PM3X_CMAKE_GENERATOR` | 强制 CMake generator |
| `PM3X_CMAKE_ARGS` | 追加给 CMake configure 的参数 |
| `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT` / `ANDROID_NDK` / `ANDROID_NDK_LATEST_HOME` | NDK 位置 |
| `PM3X_ANDROID_API` | Android API level，默认 24 |
| `PM3X_IOS_TOOLCHAIN` | ios-cmake 的 `ios.toolchain.cmake` 路径 |
| `PM3X_IOS_DEPLOYMENT_TARGET` | iOS 部署目标，默认 13.0 |

## 本地开发

```bash
git submodule update --init --recursive   # 拉取 proxmark3（约 130 MB）
cargo test --target x86_64-pc-windows-gnu # 离线模式端到端测试，无需硬件
```

`tests/offline.rs` 会在没有 Proxmark3 硬件的情况下打开离线会话，执行 `hw version`、`data -h`
和一条不存在的命令，验证「编译 → 链接 → 命令解析 → 输出抓取」，并校验流式回调确实收到了输出。

### 最小测试程序（`examples/minimal-app`）

一个刻意做小的 Tauri 2 应用，通过 path 依赖本插件，唯一目的就是验证「插件能被真正打包进 Tauri 应用」：

- 前端是静态的 `dist/index.html`，通过 Tauri IPC 调用 `pm3_version`；
- `src/lib.rs` 注册 `tauri_plugin_proxmark3::init()`、暴露 `pm3_version`，并在 `setup()` 里调用
  `client::version()` 和 `client::execute("hw version")`（桌面端）；
- `capabilities/default.json` 授权 `proxmark3:default`。

```bash
# 桌面
cargo build --release --manifest-path examples/minimal-app/Cargo.toml
# Windows：GNU 工具链必须同时作为 host toolchain
cargo +stable-x86_64-pc-windows-gnu build --release --target x86_64-pc-windows-gnu \
  --manifest-path examples/minimal-app/Cargo.toml
# Android（cdylib）
cd examples/minimal-app && cargo ndk -t arm64-v8a build --release --lib
# iOS（staticlib / cdylib）
cd examples/minimal-app && cargo build --release --target aarch64-apple-ios-sim --lib
```

可复用工作流 `.github/workflows/smoke-app.yml` 会在 Linux / macOS / Windows / Android / iOS 上
执行上述命令；`ci.yml` 每次 push 都会跑，`release.yml` 把它列进 `publish.needs`，
因此**只有五端全部编译通过才会发布 release**。

## CI / 发布

- `.github/workflows/ci.yml`：Linux / macOS / Windows(GNU) 构建 + 测试；Android 三个 ABI；iOS 模拟器与真机；`cargo package` 校验；五端最小应用（`smoke-app.yml`）
- `.github/workflows/release.yml`：推送 `v*` tag 时自动构建并发布（需 `smoke-app` 五端全部通过）：
  - `tauri-plugin-proxmark3-<version>.crate`：包含该 Proxmark3 commit 源码的 Rust 插件
  - `proxmark3-firmware-<pm3short>.zip`：`PM3BOOTROM.elf` + 各平台 `fullimage.elf`
    （`PM3RDV4`、`PM3RDV4_BTADDON`、`PM3GENERIC`、`PM3GENERIC_256`、`PM3ICOPYX_FLASH`、`PM3ULTIMATE`、`PM5`、`PM5_BWM`），
    由上游 `make PLATFORM=... PLATFORM_EXTRAS=... fullimage` 构建，ELF 名称即「平台 + 可选件」组合
  - `proxmark3-client-resources-<pm3short>.zip`：`cmdscripts / dictionaries / lualibs / luascripts / pyscripts / resources`，
    解压后按 `share/proxmark3/` 布局放置
  - `manifest.json`：机器可读清单，含 `proxmark3.commit` / `short_commit` / `commit_date` / `version`、固件列表以及每个资产的 SHA256

```bash
git tag v0.2.5
git push origin v0.2.5
```

> 固件默认不启用 standalone 模式（`STANDALONE=`）；需要各 standalone 变体时可用上游
> `tools/build_all_firmwares.sh`。所有产物都来自同一个 Proxmark3 submodule commit，
> 自动化脚本可用 `manifest.json` 的 `proxmark3.commit` 做版本匹配。

> `cargo package` 默认不会打包 git submodule 的内容，所以发布流程会把源码树复制到临时目录再打包，
> 这样发布出去的 `.crate` 是自包含、可离线构建的。

## 已知限制

- **许可证**：Proxmark3 客户端是 GPLv3。本插件静态链接了它，分发时整体需遵循 GPLv3；
  iOS App Store 的条款与 GPLv3 存在冲突，上架前请自行评估（可考虑仅内部/企业分发）。
- **Windows**：仅支持 GNU ABI；`*-msvc` 目标会在构建期报错并给出说明。
  另外 CMake 生成的目标文件路径较长，请把项目放在较短的路径下（例如 `C:\dev\pm3`），
  否则可能触发 Windows 260 字符路径限制。
- **蓝牙**：当前 CMake 选项 `SKIPBT=1`（BlueZ 是 Linux 专有依赖），需要时可通过 `PM3X_CMAKE_ARGS` 打开。
- **Android 串口**：普通 App 无权直接打开 `/dev/ttyACM*`，推荐用 `tcp:` 网络桥或离线模式。
  iOS 没有 USB Host，同理。
- 上游 `pm3_console()` 的抓取缓冲会跳过 `help` 输出，本插件改为重定向 stdout，因此一次
  `execute` 期间进程的 stdout 会被临时重定向（插件内部已串行化）。
- 命令是串行的：`execute` / `execute_stream` 共享同一个客户端实例，同一时刻只会执行一条命令。

## 目录结构

```
proxmark3/          git submodule（上游仓库，每次构建后还原）
patch/              少量上游文件的兼容副本，只在 CMake 构建期间替换进去
cmake/              仅一个 CMAKE_PROJECT_INCLUDE 注入文件（不定义工程/目标）
shim/               C bridge：open/execute/stream，以及 iOS 的 system() 替代头
src/                Rust 插件：Tauri 命令 + FFI 封装
guest-js/           TypeScript 绑定
tests/              离线模式端到端测试（含流式输出）
_temp/              本地临时环境（已 gitignore）
```
