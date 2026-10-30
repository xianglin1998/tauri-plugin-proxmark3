# tauri-plugin-proxmark3

**English** | [简体中文](README.zh-CN.md)

Compiles the [Proxmark3](https://github.com/RfidResearchGroup/proxmark3) client (`pm3`) into a
static library and exposes it as a [Tauri 2](https://v2.tauri.app/develop/plugins/) plugin, both to
the frontend and to Rust code.

Latest build artifacts (`.crate`, firmware, client resources):
[GitHub Releases](https://github.com/xianglin1998/tauri-plugin-proxmark3/releases)

- Pins Proxmark3 with a **git submodule** (`proxmark3/`)
- Uses the upstream `client/experimental_lib` (the `libpm3` exporting `pm3.h`) **directly** as a
  CMake project and never leaves the submodule modified: a small set of compatibility files in
  `patch/proxmark3/` is copied in only while CMake runs and restored afterwards; no third-party
  library is vendored into this repository
- bzip2 / lz4 are downloaded and compiled at build time by the upstream `EMBED_BZIP2` / `EMBED_LZ4`
  ExternalProjects
- iOS cross-compilation uses [leetal/ios-cmake](https://github.com/leetal/ios-cmake)
- Adds only a small C bridge (`shim/`) that works around `pm3_open()` calling `exit()` on failure
  and provides **real-time streaming output**
- One build for desktop / Android / iOS: everything is statically linked, no runtime `.so` / `.dll`
- Ships `examples/minimal-app`, a tiny Tauri app that depends on the plugin and is built on all
  five platforms in CI; a release is only published when every platform compiled

## Support matrix

| Platform | Status | Transports |
| --- | --- | --- |
| Android (arm64-v8a / armeabi-v7a / x86_64) | ✅ CI build | offline mode, `tcp:` / `udp:` network bridge |
| iOS (arm64 device / arm64 simulator) | ✅ CI build (ios-cmake) | offline mode, `tcp:` / `udp:` network bridge |
| Linux x86_64 | ✅ CI build + test | serial, `tcp:` / `udp:` / `bt:` (Linux builds) |
| macOS arm64 | ✅ CI build + test | serial, `tcp:` / `udp:` |
| Windows x86_64 (GNU ABI / MinGW) | ✅ CI build + test | serial (`COM3`), `tcp:` / `udp:` |

> Windows only works with `x86_64-pc-windows-gnu`. The upstream client is GNU-flavoured C
> (`__attribute__((packed))`, POSIX headers, pthread), which MSVC / clang-cl cannot compile, so the
> crate aborts with a clear error on `*-msvc` targets.

## How it works

```
┌────────────────────────────┐
│ Frontend (JS/TS)           │  invoke('plugin:proxmark3|execute')
└─────────────┬──────────────┘
              │ Tauri IPC (permission: proxmark3:default)
┌─────────────▼──────────────┐
│ Rust plugin (this crate)   │  global mutex + spawn_blocking; streaming via events
└─────────────┬──────────────┘
              │ C ABI (shim/pm3_bridge.c)
┌─────────────▼──────────────┐
│ libpm3 (static)            │  proxmark3/client/experimental_lib: pm3rrg_rdv4_static
│ + mbedtls / lua / jansson… │  plus the static libraries from the upstream deps
└────────────────────────────┘
```

What `build.rs` does:

1. copies the compatibility files from `patch/proxmark3/` over the submodule (they are restored
   when the build script exits), then runs
   `cmake -S proxmark3/client/experimental_lib -B <build>` with
   `SKIPBT/SKIPPYTHON/SKIPREADLINE/SKIPLINENOISE/SKIPGD` and `EMBED_*` for bzip2 / lz4;
2. injects a few **compiler options** through
   `-DCMAKE_PROJECT_INCLUDE=cmake/pm3x_configure.cmake`:
   - Android x86/x86_64 needs `-mavx2` / `-mavx512f` for `cipher_bs_avx*.c` (upstream adds the
     target pragmas only for non-Android builds);
   - iOS has no `system()`, so `-include shim/ios_system.h` redirects it to an inline stub instead
     of patching the source;
   - upstream builds with `-Werror`; the error promotion is dropped (the consumer controls the host
     compiler version).

   That file creates no project, no target, and does not touch the upstream build files;
3. runs `cmake --build ... --target pm3rrg_rdv4_static`;
4. passes the produced `.a` files to `cargo:rustc-link-lib` in dependency order and adds the system
   libraries (zlib / pthread / ws2_32 / frameworks) per platform;
5. compiles `shim/pm3_bridge.c` with `cc`.

### Where bzip2 / lz4 come from

- The Proxmark3 repository does **not** bundle bzip2 / lz4 (`client/deps` has no such directories);
- the upstream `client/experimental_lib/CMakeLists.txt` provides `EMBED_BZIP2` / `EMBED_LZ4`, which
  fetch the sources from `android.googlesource.com` with ExternalProject and build them with `make`;
- that path needs a POSIX shell (`mkdir -p`, pipes), so it is used on **Linux / macOS / Android /
  iOS**;
- native Windows CMake generators execute commands with `cmd.exe`, where `mkdir -p` does not work,
  so Windows uses the MSYS2 `mingw-w64-ucrt-x86_64-bzip2` / `-lz4` packages instead (see
  “Build requirements”).

## Installation

```toml
# the application crate's Cargo.toml
[dependencies]
tauri-plugin-proxmark3 = { git = "https://github.com/xianglin1998/tauri-plugin-proxmark3", tag = "v0.2.4" }
# or download the .crate from a GitHub Release and use a path dependency
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

Grant the permission in `src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "windows": ["main"],
  "permissions": ["core:default", "proxmark3:default"]
}
```

### Frontend

```ts
import { connect, execute, executeStream, status, disconnect } from 'tauri-plugin-proxmark3-api';

// offline mode (no hardware) or auto-detect the first device
await connect();

// explicit transport:
//   serial    "/dev/ttyACM0" / "COM3"
//   network   "tcp:192.168.1.20:18888" / "udp:192.168.1.20:18888"
//   bluetooth "bt:AA:BB:CC:DD:EE:FF" (Linux + BlueZ builds only)
await connect('tcp:192.168.1.20:18888');

// the complete output at once
const { status: code, output } = await execute('hw version');

// real-time output: every chunk while the command is still running
await executeStream('hf search', (chunk) => {
  console.log('streamed:', chunk);
});
```

Streaming uses the Tauri event `proxmark3://output` (payload: `{ chunk: string }`);
`executeStream()` subscribes and unsubscribes automatically.

## API

| Command | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `connect` | `port?: string \| null` | `SessionInfo` | open a session; `null` means offline / auto-detect |
| `disconnect` | – | `SessionInfo` | close the session |
| `execute` | `command: string` | `CommandOutput` | run a client command; opens an offline session on first use |
| `execute_stream` | `command: string` | `CommandOutput` | run a command and stream its output through `proxmark3://output` |
| `status` | – | `SessionInfo` | current session state |
| `version` | – | `string` | client version compiled into the library |

```ts
interface SessionInfo { open: boolean; offline: boolean; port: string; version: string }
interface CommandOutput { status: number; output: string; durationMs: number }
```

Rust code can also call the plugin without going through the Tauri command layer:

```rust
use tauri_plugin_proxmark3::client;

client::connect(None)?;
let out = client::execute("hf search")?;
println!("{}", out.output);

// streaming
client::execute_stream("hf search", |chunk| print!("{chunk}"))?;
```

## Build requirements

| Dependency | Notes |
| --- | --- |
| Rust stable | ≥ 1.77.2 |
| CMake ≥ 3.19 | upstream needs 3.16; this plugin injects options with `cmake_language(DEFER)`, which needs 3.19 |
| Ninja | optional but recommended (used when found), otherwise the platform default generator |
| C/C++ compiler | Linux/macOS: gcc/clang; Android: NDK clang; iOS: Apple clang; Windows: MSYS2 UCRT64 gcc |
| git / make / tar | download and build of the bzip2 / lz4 ExternalProjects (not needed on Windows) |
| zlib | when found, the upstream zlib support is enabled (Linux: `zlib1g-dev`) |
| Linux desktop | Tauri itself needs GTK/WebKitGTK: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` |

Windows (MSYS2 UCRT64):

```powershell
# in MSYS2:
#   pacman -S mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-cmake \
#             mingw-w64-ucrt-x86_64-ninja mingw-w64-ucrt-x86_64-zlib \
#             mingw-w64-ucrt-x86_64-bzip2 mingw-w64-ucrt-x86_64-lz4
rustup target add x86_64-pc-windows-gnu
$env:PM3X_MINGW_BIN = "C:\msys64\ucrt64\bin"   # if not in a default location
cargo build --target x86_64-pc-windows-gnu
```

Android (NDK installed, `cargo install cargo-ndk`):

```bash
export ANDROID_NDK_HOME=$ANDROID_SDK_ROOT/ndk/27.0.12077973
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o jniLibs build --release
```

iOS (macOS + Xcode + [ios-cmake](https://github.com/leetal/ios-cmake)):

```bash
git clone --depth 1 https://github.com/leetal/ios-cmake.git _temp/ios-cmake
export PM3X_IOS_TOOLCHAIN="$PWD/_temp/ios-cmake/ios.toolchain.cmake"

rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios
```

> When `PM3X_IOS_TOOLCHAIN` is not set, `build.rs` clones ios-cmake into the system temp directory
> automatically (requires access to GitHub).

### Environment variables

| Variable | Purpose |
| --- | --- |
| `CMAKE` | cmake executable to use |
| `PM3X_MINGW_BIN` | directory with gcc/ar on Windows (defaults to `C:\msys64\ucrt64\bin` etc.) |
| `PM3X_CMAKE_GENERATOR` | force a CMake generator |
| `PM3X_CMAKE_ARGS` | extra arguments for the CMake configure step |
| `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT` / `ANDROID_NDK` / `ANDROID_NDK_LATEST_HOME` | NDK location |
| `PM3X_ANDROID_API` | Android API level, default 24 |
| `PM3X_IOS_TOOLCHAIN` | path to ios-cmake's `ios.toolchain.cmake` |
| `PM3X_IOS_DEPLOYMENT_TARGET` | iOS deployment target, default 13.0 |

## Local development

```bash
git submodule update --init --recursive   # fetch proxmark3 (about 130 MB)
cargo test --target x86_64-pc-windows-gnu # offline end-to-end test, no hardware needed
```

`tests/offline.rs` opens an offline session without any Proxmark3 hardware, runs `hw version`,
`data -h` and an unknown command, and checks the whole chain “compile → link → command parser →
output capture”, including that the streaming callback really receives output.

### Minimal app (`examples/minimal-app`)

A deliberately tiny Tauri 2 application that depends on the plugin through a path dependency. Its
only purpose is to prove that the plugin can be packaged into a real app:

- the frontend is a static `dist/index.html` that calls `pm3_version` through the Tauri IPC layer;
- `src/lib.rs` registers `tauri_plugin_proxmark3::init()`, exposes `pm3_version`, and in `setup()`
  calls `client::version()` and `client::execute("hw version")` (desktop);
- `capabilities/default.json` grants `proxmark3:default`.

```bash
# desktop
cargo build --release --manifest-path examples/minimal-app/Cargo.toml
# Windows: the GNU toolchain has to be the host toolchain as well
cargo +stable-x86_64-pc-windows-gnu build --release --target x86_64-pc-windows-gnu \
  --manifest-path examples/minimal-app/Cargo.toml
# Android (cdylib)
cd examples/minimal-app && cargo ndk -t arm64-v8a build --release --lib
# iOS (staticlib / cdylib)
cd examples/minimal-app && cargo build --release --target aarch64-apple-ios-sim --lib
```

The reusable workflow `.github/workflows/smoke-app.yml` runs exactly these commands for Linux,
macOS, Windows, Android and iOS. `ci.yml` runs it on every push and `release.yml` lists it in
`publish.needs`, so **no release is created unless all five platforms compile**.

## CI / release

- `.github/workflows/ci.yml`: Linux / macOS / Windows(GNU) build + test; three Android ABIs; iOS
  simulator and device; `cargo package` check; the five-platform minimal app (`smoke-app.yml`)
- `.github/workflows/release.yml`: triggered by a `v*` tag, builds and publishes (only after
  `smoke-app` succeeded on all five platforms):
  - `tauri-plugin-proxmark3-<version>.crate`: the Rust plugin with the pinned Proxmark3 sources
  - `proxmark3-firmware-<pm3short>.zip`: `PM3BOOTROM.elf` plus one `fullimage.elf` per platform
    (`PM3RDV4`, `PM3RDV4_BTADDON`, `PM3GENERIC`, `PM3GENERIC_256`, `PM3ICOPYX_FLASH`,
    `PM3ULTIMATE`, `PM5`, `PM5_BWM`), built with the upstream
    `make PLATFORM=... PLATFORM_EXTRAS=... fullimage`; the ELF name is the
    “platform + optional components” combination
  - `proxmark3-client-resources-<pm3short>.zip`: `cmdscripts / dictionaries / lualibs / luascripts /
    pyscripts / resources`, packed as `share/proxmark3/...`
  - `manifest.json`: machine readable list with `proxmark3.commit` / `short_commit` /
    `commit_date` / `version`, the firmware matrix and the SHA256 of every asset

```bash
git tag v0.2.4
git push origin v0.2.4
```

> Firmware is built without a standalone mode (`STANDALONE=`); use the upstream
> `tools/build_all_firmwares.sh` if you need the standalone variants. All artifacts come from the
> same Proxmark3 submodule commit, and automation can match on `proxmark3.commit` in
> `manifest.json`.

> `cargo package` does not include git submodule content by default, so the release flow copies the
> source tree to a temporary directory before packaging; the published `.crate` is self-contained
> and can be built offline.

## Known limitations

- **License**: the Proxmark3 client is GPLv3. This plugin links it statically, so the combined work
  must be distributed under GPLv3; the App Store terms conflict with GPLv3, evaluate before
  shipping to the iOS App Store (internal/enterprise distribution is usually fine).
- **Windows**: GNU ABI only; `*-msvc` targets fail at build time with an explanation. CMake also
  generates long object paths, so keep the project in a short path (for example `C:\dev\pm3`) to
  stay under the Windows 260 character limit.
- **Bluetooth**: `SKIPBT=1` for now (BlueZ is a Linux-only dependency); enable it through
  `PM3X_CMAKE_ARGS` if needed.
- **Android serial**: a normal app cannot open `/dev/ttyACM*`; use the `tcp:` network bridge or
  offline mode. iOS has no USB host either.
- The upstream `pm3_console()` capture buffer skips `help` output, so this plugin redirects stdout;
  during one `execute` the process stdout is temporarily redirected (commands are serialised).
- Commands are serialised: `execute` / `execute_stream` share one client instance, so only one
  command runs at a time.

## Repository layout

```
proxmark3/          git submodule (upstream repository, restored after every build)
patch/              compatibility copies of a few upstream files, applied only while CMake runs
cmake/              a single CMAKE_PROJECT_INCLUDE file (no project, no targets)
shim/               C bridge: open/execute/stream, plus the iOS system() replacement header
src/                Rust plugin: Tauri commands + FFI wrapper
guest-js/           TypeScript bindings
tests/              offline end-to-end tests (including streaming)
_temp/              local temporary environment (gitignored)
```
