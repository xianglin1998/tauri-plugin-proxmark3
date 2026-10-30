//! Build script for the Proxmark3 client plugin.
//!
//! The upstream Proxmark3 client library (`client/experimental_lib`, target
//! `pm3rrg_rdv4_static`) is configured and built directly with CMake; no copy
//! of its build files and no vendored third party sources are kept in this
//! crate.  The only addition is `cmake/pm3x_configure.cmake`, which CMake
//! injects through `CMAKE_PROJECT_INCLUDE` to set the few flags the upstream
//! project cannot know about (Android AVX sources, iOS `system()`, `-Wno-error`).
//!
//! bzip2 and lz4 are the two libraries the client always needs.  On POSIX
//! targets the upstream `EMBED_BZIP2`/`EMBED_LZ4` ExternalProject downloads and
//! builds them (needs git, make, tar and a POSIX shell).  That path cannot run
//! under a native Windows `cmd.exe`, so on Windows the MSYS2 packages provide
//! them instead.
//!
//! Environment variables:
//!   CMAKE                     cmake executable (default: `cmake` from PATH)
//!   PM3X_MINGW_BIN            directory with gcc/g++ on Windows/MinGW
//!   PM3X_CMAKE_GENERATOR      override the CMake generator
//!   PM3X_CMAKE_ARGS           extra arguments for the CMake configure step
//!   ANDROID_NDK_HOME / ...    Android NDK location (see `android_ndk_dir`)
//!   PM3X_ANDROID_API          Android API level (default: 24)
//!   PM3X_IOS_TOOLCHAIN        path of leetal/ios-cmake's ios.toolchain.cmake
//!                             (default: cloned into the temp dir on demand)
//!   PM3X_IOS_DEPLOYMENT_TARGET minimum iOS version (default: 13.0)

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COMMANDS: &[&str] = &[
    "connect",
    "disconnect",
    "execute",
    "execute_stream",
    "status",
    "version",
    "input",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let target = env::var("TARGET").expect("TARGET");
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS");
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH");

    for change in [
        "shim/pm3_bridge.c",
        "shim/pm3_bridge.h",
        "shim/ios_system.h",
        "cmake/pm3x_configure.cmake",
        "proxmark3/client/src",
        "proxmark3/client/include",
        "proxmark3/client/deps",
        "proxmark3/common",
        "proxmark3/include",
        "proxmark3/common_fpga",
        "proxmark3/tools/mkversion.sh",
        "patch/proxmark3",
    ] {
        println!("cargo:rerun-if-changed={change}");
    }
    for var in [
        "CMAKE",
        "PM3X_MINGW_BIN",
        "PM3X_CMAKE_GENERATOR",
        "PM3X_CMAKE_ARGS",
        "PM3X_ANDROID_API",
        "PM3X_IOS_TOOLCHAIN",
        "PM3X_IOS_DEPLOYMENT_TARGET",
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "ANDROID_NDK",
        "ANDROID_NDK_LATEST_HOME",
        "ANDROID_SDK_ROOT",
        "ANDROID_HOME",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    let upstream_dir = manifest_dir.join("proxmark3/client/experimental_lib");
    if !upstream_dir.join("CMakeLists.txt").exists() {
        panic!(
            "\n\n\
             The Proxmark3 submodule is missing.\n\
             Fetch it once with:\n\n    \
             git submodule update --init --recursive\n\n\
             (expected sources in {})\n",
            manifest_dir.join("proxmark3").display()
        );
    }

    // Apply the small set of compatibility patches to the upstream client
    // while CMake runs, then restore the submodule files. The patches are kept
    // in this repository so the Proxmark3 submodule itself stays upstream.
    let _patched_sources = SourcePatches::apply(&manifest_dir);

    // CMake object paths inside rustc's target directory can exceed the 260
    // character Windows limit, so the CMake build tree lives on a short path.
    let build_dir = cmake_build_dir(&out_dir);

    let mut configure = Command::new(cmake_program());
    configure
        .arg("-S")
        .arg(&upstream_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON")
        .arg(format!(
            "-DCMAKE_PROJECT_INCLUDE={}",
            manifest_dir.join("cmake/pm3x_configure.cmake").display()
        ))
        .args([
            "-DSKIPBT=1",
            "-DSKIPPYTHON=1",
            "-DSKIPREADLINE=1",
            "-DSKIPLINENOISE=1",
            "-DSKIPGD=1",
            "-DSKIPJANSSONSYSTEM=1",
            "-DSKIPWHEREAMISYSTEM=1",
        ]);

    if let Ok(extra) = env::var("PM3X_CMAKE_ARGS") {
        for arg in extra.split_whitespace() {
            configure.arg(arg);
        }
    }

    let mut path_prefix: Vec<PathBuf> = Vec::new();
    let mut generator: Option<String> = None;

    match target_os.as_str() {
        "windows" => {
            if target_env == "msvc" {
                panic!(
                    "\n\n\
                     The Proxmark3 client can only be compiled with a GNU toolchain \
                     (it relies on POSIX headers and GNU attributes).\n\
                     Build this crate with the `x86_64-pc-windows-gnu` target, e.g.:\n\n    \
                     rustup target add x86_64-pc-windows-gnu\n    \
                     cargo build --target x86_64-pc-windows-gnu\n\n\
                     `PM3X_MINGW_BIN` may be used to point at the MSYS2 UCRT64 bin directory.\n"
                );
            }
            let mingw = mingw_bin_dir().unwrap_or_else(|| {
                panic!(
                    "\n\n\
                     Could not find an MSYS2 UCRT64 toolchain (gcc).\n\
                     Install it (https://www.msys2.org/) and, if it lives outside the usual \
                     locations, set PM3X_MINGW_BIN to its `ucrt64\\bin` directory.\n"
                )
            });
            let root = mingw.parent().map(Path::to_path_buf);
            configure
                .arg(format!("-DCMAKE_C_COMPILER={}", gcc_path(&mingw).display()))
                .arg(format!("-DCMAKE_CXX_COMPILER={}", gxx_path(&mingw).display()));
            path_prefix.push(mingw.clone());
            if let Some(root) = root {
                let usr_bin = root.join("usr").join("bin");
                if usr_bin.exists() {
                    path_prefix.push(usr_bin); // sh.exe for mkversion.sh
                }
            }
            generator = Some("Ninja".to_string());
            // bzip2/lz4 come from the MSYS2 packages here: the upstream
            // ExternalProject steps rely on a POSIX shell.
        }
        "android" => {
            let ndk = android_ndk_dir().unwrap_or_else(|| {
                panic!(
                    "\n\n\
                     Could not locate the Android NDK. Set ANDROID_NDK_HOME to the NDK \
                     directory (for example $ANDROID_SDK_ROOT/ndk/27.0.12077973).\n"
                )
            });
            let abi = match target_arch.as_str() {
                "aarch64" => "arm64-v8a",
                "arm" => "armeabi-v7a",
                "x86_64" => "x86_64",
                "x86" => "x86",
                other => panic!("unsupported Android architecture: {other}"),
            };
            let api = env::var("PM3X_ANDROID_API").unwrap_or_else(|_| "24".to_string());
            configure
                .arg(format!(
                    "-DCMAKE_TOOLCHAIN_FILE={}",
                    ndk.join("build/cmake/android.toolchain.cmake").display()
                ))
                .arg(format!("-DANDROID_ABI={abi}"))
                .arg(format!("-DANDROID_PLATFORM=android-{api}"))
                .args(["-DEMBED_BZIP2=ON", "-DEMBED_LZ4=ON"]);
        }
        "ios" => {
            let (platform, arch, sdk_name, simulator) = match target.as_str() {
                "aarch64-apple-ios" => ("OS64", "arm64", "iphoneos", false),
                "aarch64-apple-ios-sim" => ("SIMULATORARM64", "arm64", "iphonesimulator", true),
                "x86_64-apple-ios" => ("SIMULATOR64", "x86_64", "iphonesimulator", true),
                other => panic!("unsupported iOS target: {other}"),
            };
            let toolchain = ios_toolchain(&manifest_dir);
            let sdk = xcrun_sdk_path(sdk_name);
            let deployment = env::var("PM3X_IOS_DEPLOYMENT_TARGET")
                .unwrap_or_else(|_| "13.0".to_string());

            // The upstream ExternalProject builds bzip2/lz4 with plain make,
            // so the cross compilation flags have to reach that make call.
            // Upstream only sets CFLAGS_EXTERNAL_LIB for Android, therefore
            // the value is passed here and survives for iOS.
            let mut cflags = format!("CFLAGS=-arch {arch} -isysroot {sdk} -O2 -w");
            if simulator {
                cflags.push_str(&format!(" -mios-simulator-version-min={deployment}"));
            } else {
                cflags.push_str(&format!(" -mios-version-min={deployment}"));
            }

            configure
                .arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", toolchain.display()))
                .arg(format!("-DPLATFORM={platform}"))
                .arg(format!("-DDEPLOYMENT_TARGET={deployment}"))
                .arg("-DENABLE_BITCODE=OFF")
                .arg(format!(
                    "-DPM3X_IOS_SYSTEM_HEADER={}",
                    manifest_dir.join("shim/ios_system.h").display()
                ))
                .args(["-DEMBED_BZIP2=ON", "-DEMBED_LZ4=ON"])
                .arg(format!("-DCFLAGS_EXTERNAL_LIB={cflags}"));
        }
        _ => {
            configure.args(["-DEMBED_BZIP2=ON", "-DEMBED_LZ4=ON"]);
        }
    }

    if generator.is_none() {
        generator = cmake_generator();
    }
    if let Some(generator) = generator {
        configure.arg("-G").arg(generator);
    }

    if !path_prefix.is_empty() {
        let path = path_with_prefix(&path_prefix);
        configure.env("PATH", &path);
    }

    run(&mut configure, "configure the Proxmark3 client");

    let mut build = Command::new(cmake_program());
    build
        .arg("--build")
        .arg(&build_dir)
        .arg("--target")
        .arg("pm3rrg_rdv4_static")
        .arg("-j")
        .arg(num_jobs().to_string());
    if !path_prefix.is_empty() {
        let path = path_with_prefix(&path_prefix);
        build.env("PATH", &path);
    }
    run(&mut build, "build the Proxmark3 client library");

    // ------------------------------------------------------------------
    // C bridge (compiled first so GNU ld sees the calling object before the
    // client archives)
    // ------------------------------------------------------------------
    let mut bridge = cc::Build::new();
    bridge.file(manifest_dir.join("shim/pm3_bridge.c"));
    for dir in client_include_dirs(&manifest_dir) {
        bridge.include(dir);
    }
    bridge.include(manifest_dir.join("shim"));
    bridge.flag_if_supported("-D_ISOC99_SOURCE");
    bridge.flag_if_supported("-Wno-old-style-declaration");
    bridge.warnings(true);
    if let Some(mingw) = mingw_bin_dir_if_windows(&target_os) {
        if target_env == "gnu" {
            bridge.compiler(gcc_path(&mingw));
            bridge.archiver(mingw.join(exe("ar")));
        }
    }
    bridge.compile("pm3bridge");

    // ------------------------------------------------------------------
    // Link the client archives
    // ------------------------------------------------------------------
    let archives = client_archives(&build_dir, &target_os);
    let mut search_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for archive in &archives {
        if let Some(dir) = archive.parent() {
            search_dirs.insert(dir.to_path_buf());
        }
        println!(
            "cargo:rustc-link-lib=static={}",
            archive_stem(archive)
        );
    }
    for dir in search_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    // Rustc only searches explicit -L directories for `static=` libraries.
    // The MSYS2 bzip2/lz4/winpthread static archives live next to the MinGW
    // toolchain, so add that lib directory for the final link as well.
    if target_os == "windows" {
        if let Some(mingw) = mingw_bin_dir() {
            if let Some(root) = mingw.parent() {
                println!(
                    "cargo:rustc-link-search=native={}",
                    root.join("lib").display()
                );
            }
        }
    }

    if target_os == "android" && (target_arch == "x86_64" || target_arch == "x86") {
        if let Some(ndk) = android_ndk_dir() {
            let arch = if target_arch == "x86" { "i686" } else { "x86_64" };
            let name = format!("libclang_rt.builtins-{arch}-android.a");
            if let Some(archive) = find_file(&ndk.join("toolchains/llvm/prebuilt"), &name) {
                if let Some(dir) = archive.parent() {
                    println!("cargo:rustc-link-search=native={}", dir.display());
                    println!("cargo:rustc-link-lib=static=clang_rt.builtins-{arch}-android");
                }
            }
        }
    }

    for lib in system_libs(&build_dir, &target_os) {
        if let Some(framework) = lib.strip_prefix("framework:") {
            println!("cargo:rustc-link-lib=framework={framework}");
        } else {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    println!("cargo:rustc-env=PM3X_TARGET_TRIPLE={target}");
}

// ---------------------------------------------------------------------------
// archives / libraries
// ---------------------------------------------------------------------------

/// Archives produced by `pm3rrg_rdv4_static`, in dependency order (GNU ld
/// resolves archives left to right).
fn client_archives(build_dir: &Path, target_os: &str) -> Vec<PathBuf> {
    let mut archives = vec![
        build_dir.join("libpm3rrg_rdv4_static.a"),
        build_dir.join("deps/libpm3rrg_rdv4_mbedtls.a"),
        build_dir.join("deps/libpm3rrg_rdv4_cliparser.a"),
        build_dir.join("deps/libpm3rrg_rdv4_lua.a"),
        build_dir.join("deps/libpm3rrg_rdv4_tinycbor.a"),
        build_dir.join("deps/libpm3rrg_rdv4_amiibo.a"),
        build_dir.join("deps/libpm3rrg_rdv4_reveng.a"),
        build_dir.join("deps/libpm3rrg_rdv4_hardnested.a"),
        build_dir.join("deps/libpm3rrg_rdv4_id48.a"),
        build_dir.join("deps/libpm3rrg_rdv4_mqtt.a"),
        build_dir.join("deps/libpm3rrg_rdv4_jansson.a"),
        build_dir.join("deps/libpm3rrg_rdv4_whereami.a"),
    ];
    if target_os != "windows" {
        // Built by the upstream EMBED_BZIP2 / EMBED_LZ4 ExternalProjects.
        archives.push(build_dir.join("deps/bzip2/src/bzip2/libbz2.a"));
        archives.push(build_dir.join("deps/lz4/src/lz4/lib/liblz4.a"));
    }

    for archive in &archives {
        if !archive.exists() {
            panic!(
                "\n\n\
                 Expected the Proxmark3 build to produce {}.\n\
                 If the upstream build changed its target names, update \
                 `client_archives()` in build.rs.\n",
                archive.display()
            );
        }
    }
    archives
}

fn system_libs(build_dir: &Path, target_os: &str) -> Vec<String> {
    let mut libs: Vec<String> = Vec::new();
    match target_os {
        "windows" => {
            // MSYS2 UCRT64 packages provide bzip2 and lz4 (the upstream
            // ExternalProject steps need a POSIX shell and do not run on cmd).
            // Link the static archives explicitly so the release executable
            // does not depend on libbz2-1.dll, liblz4.dll or libwinpthread-1.dll.
            libs.extend(["ws2_32"].map(str::to_string));
            libs.extend(["static=pthread", "static=bz2", "static=lz4"].map(str::to_string));
        }
        "android" => libs.extend(["m"].map(str::to_string)),
        "ios" | "macos" => {
            libs.extend(["m"].map(str::to_string));
            libs.push("framework:Foundation".into());
            if target_os == "ios" {
                libs.push("framework:UIKit".into());
            } else {
                libs.push("framework:AppKit".into());
            }
        }
        _ => libs.extend(["m", "pthread"].map(str::to_string)),
    }

    if cmake_cache_has_library(build_dir, "ZLIB_LIBRARY")
        || cmake_cache_has_library(build_dir, "ZLIB_LIBRARY_RELEASE")
    {
        libs.push(if target_os == "windows" { "static=z" } else { "z" }.into());
    }
    libs
}

/// Reads `Name:FILEPATH=...` from CMakeCache.txt and reports whether the
/// library was found.
fn cmake_cache_has_library(build_dir: &Path, name: &str) -> bool {
    let cache = build_dir.join("CMakeCache.txt");
    let Ok(content) = std::fs::read_to_string(&cache) else {
        return false;
    };
    content.lines().any(|line| {
        line.strip_prefix(&format!("{name}:"))
            .and_then(|rest| rest.split_once('='))
            .map(|(_, value)| !value.trim().is_empty() && !value.contains("NOTFOUND"))
            .unwrap_or(false)
    })
}

fn client_include_dirs(manifest_dir: &Path) -> Vec<PathBuf> {
    let proxmark3 = manifest_dir.join("proxmark3");
    let deps = proxmark3.join("client/deps");
    vec![
        proxmark3.join("common"),
        proxmark3.join("common_fpga"),
        proxmark3.join("include"),
        proxmark3.join("client/src"),
        proxmark3.join("client/include"),
        proxmark3.join("common/mbedtls"),
        deps.join("jansson"),
        deps.join("cliparser"),
        deps.join("liblua"),
        deps.join("tinycbor"),
        deps.join("amiitool"),
        deps.join("reveng"),
        deps.join("hardnested"),
        deps.join("id48/public"),
        deps.join("mqtt"),
        deps.join("vec"),
        deps.join("whereami"),
    ]
}

// ---------------------------------------------------------------------------
// toolchains
// ---------------------------------------------------------------------------

fn cmake_build_dir(out_dir: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    out_dir.hash(&mut hasher);
    let digest = format!("{:016x}", hasher.finish());

    #[cfg(windows)]
    {
        std::env::temp_dir().join(format!("pm3x-{digest}"))
    }
    #[cfg(not(windows))]
    {
        let _ = digest;
        out_dir.join("pm3build")
    }
}

fn cmake_program() -> String {
    env::var("CMAKE").unwrap_or_else(|_| "cmake".to_string())
}

fn cmake_generator() -> Option<String> {
    if let Ok(value) = env::var("PM3X_CMAKE_GENERATOR") {
        return Some(value);
    }
    let probe = Command::new(if cfg!(windows) { "ninja.exe" } else { "ninja" })
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match probe {
        Ok(status) if status.success() => Some("Ninja".to_string()),
        _ => None,
    }
}

fn ios_toolchain(manifest_dir: &Path) -> PathBuf {
    if let Ok(value) = env::var("PM3X_IOS_TOOLCHAIN") {
        let path = PathBuf::from(value);
        return if path.is_dir() {
            path.join("ios.toolchain.cmake")
        } else {
            path
        };
    }

    let cache = std::env::temp_dir().join("pm3x-ios-cmake");
    let toolchain = cache.join("ios.toolchain.cmake");
    if !toolchain.exists() {
        let _ = std::fs::remove_dir_all(&cache);
        let status = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                "https://github.com/leetal/ios-cmake.git",
            ])
            .arg(&cache)
            .status();
        match status {
            Ok(status) if status.success() => {}
            _ => panic!(
                "\n\n\
                 Could not fetch leetal/ios-cmake.\n\
                 Clone it manually and point PM3X_IOS_TOOLCHAIN at \
                 ios.toolchain.cmake:\n\n    \
                 git clone --depth 1 https://github.com/leetal/ios-cmake.git _temp/ios-cmake\n    \
                 export PM3X_IOS_TOOLCHAIN={}/_temp/ios-cmake/ios.toolchain.cmake\n",
                manifest_dir.display()
            ),
        }
    }
    toolchain
}

fn xcrun_sdk_path(sdk: &str) -> String {
    let output = Command::new("xcrun")
        .args(["--sdk", sdk, "--show-sdk-path"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run xcrun --sdk {sdk}: {error}"));
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || path.is_empty() {
        panic!("xcrun could not locate the {sdk} SDK");
    }
    path
}

fn num_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1)
}

fn path_with_prefix(prefix: &[PathBuf]) -> std::ffi::OsString {
    let mut paths: Vec<PathBuf> = prefix.to_vec();
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).expect("valid PATH")
}

// ---------------------------------------------------------------------------
// Upstream source patches
// ---------------------------------------------------------------------------

/// Files copied from `patch/proxmark3/` into the submodule for the duration of
/// the CMake build. They keep the submodule checkout itself untouched.
const SOURCE_PATCHES: &[&str] = &[
    "client/deps/hardnested/hardnested_bf_core.h",
    "client/deps/mqtt/mqtt_pal.h",
    "client/experimental_lib/CMakeLists.txt",
    "client/src/loclass/cipher_bs_avx2.c",
    "client/src/loclass/cipher_bs_avx512.c",
    "client/src/util.c",
];

struct SourcePatches {
    originals: Vec<(PathBuf, Vec<u8>)>,
}

impl SourcePatches {
    fn apply(manifest_dir: &Path) -> Self {
        let mut originals = Vec::new();
        for relative in SOURCE_PATCHES {
            let source = manifest_dir.join("patch/proxmark3").join(relative);
            if !source.exists() {
                continue;
            }
            let target = manifest_dir.join("proxmark3").join(relative);
            let original = fs::read(&target)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", target.display()));
            let replacement = fs::read(&source)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", source.display()));
            if original != replacement {
                fs::write(&target, &replacement)
                    .unwrap_or_else(|error| panic!("failed to patch {}: {error}", target.display()));
            }
            originals.push((target, original));
        }
        Self { originals }
    }
}

impl Drop for SourcePatches {
    fn drop(&mut self) {
        for (target, original) in &self.originals {
            if let Err(error) = fs::write(target, original) {
                eprintln!("warning: failed to restore {}: {error}", target.display());
            }
        }
    }
}

fn run(command: &mut Command, what: &str) {
    let printable = format!("{command:?}");
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to {what} ({printable}): {error}"));
    if !status.success() {
        panic!("failed to {what} ({printable}): {status}");
    }
}

fn archive_stem(path: &Path) -> String {
    let mut name = path
        .file_stem()
        .unwrap_or_else(|| panic!("invalid archive path: {}", path.display()))
        .to_string_lossy()
        .into_owned();
    if let Some(stripped) = name.strip_prefix("lib") {
        name = stripped.to_string();
    }
    name
}

// ---------------------------------------------------------------------------
// Windows / MinGW
// ---------------------------------------------------------------------------

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn mingw_bin_dir_if_windows(target_os: &str) -> Option<PathBuf> {
    if target_os == "windows" {
        mingw_bin_dir()
    } else {
        None
    }
}

fn mingw_bin_dir() -> Option<PathBuf> {
    if let Ok(value) = env::var("PM3X_MINGW_BIN") {
        let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
        let dir = PathBuf::from(value);
        if has_gcc(&dir) {
            return Some(dir);
        }
    }
    [
        r"C:\msys64\ucrt64\bin",
        r"C:\msys2\ucrt64\bin",
        r"C:\UserProgram\msys2\ucrt64\bin",
        r"C:\tools\msys64\ucrt64\bin",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|dir| has_gcc(dir))
}

fn has_gcc(dir: &Path) -> bool {
    dir.join("gcc.exe").exists() || dir.join("x86_64-w64-mingw32-gcc.exe").exists()
}

fn gcc_path(dir: &Path) -> PathBuf {
    let direct = dir.join("gcc.exe");
    if direct.exists() {
        direct
    } else {
        dir.join("x86_64-w64-mingw32-gcc.exe")
    }
}

fn gxx_path(dir: &Path) -> PathBuf {
    let direct = dir.join("g++.exe");
    if direct.exists() {
        direct
    } else {
        dir.join("x86_64-w64-mingw32-g++.exe")
    }
}

// ---------------------------------------------------------------------------
// Android
// ---------------------------------------------------------------------------

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().and_then(|value| value.to_str()) == Some(name) {
                return Some(path);
            }
        }
    }
    None
}

fn android_ndk_dir() -> Option<PathBuf> {
    for var in [
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "ANDROID_NDK",
        "ANDROID_NDK_LATEST_HOME",
    ] {
        if let Ok(value) = env::var(var) {
            if !value.is_empty() {
                let dir = PathBuf::from(value);
                if dir.join("build/cmake/android.toolchain.cmake").exists() {
                    return Some(dir);
                }
            }
        }
    }

    let mut sdk_roots: Vec<PathBuf> = Vec::new();
    for var in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Ok(value) = env::var(var) {
            if !value.is_empty() {
                sdk_roots.push(PathBuf::from(value));
            }
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        sdk_roots.push(PathBuf::from(local).join("Android").join("Sdk"));
    }
    if let Some(home) = env::var_os("HOME") {
        sdk_roots.push(PathBuf::from(home).join("Android").join("Sdk"));
    }

    for sdk in sdk_roots {
        let ndk_dir = sdk.join("ndk");
        let Ok(entries) = std::fs::read_dir(&ndk_dir) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("build/cmake/android.toolchain.cmake").exists())
            .collect();
        versions.sort();
        if let Some(latest) = versions.pop() {
            return Some(latest);
        }
    }

    None
}
