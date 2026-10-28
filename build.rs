//! Build script for the Proxmark3 client plugin.
//!
//! It compiles the upstream Proxmark3 client (the `client/experimental_lib`
//! CMake project of the git submodule) into a set of static libraries, then
//! compiles the Rust-facing C bridge (`shim/pm3_bridge.c`) that wraps them.
//!
//! Everything a caller needs at runtime is linked statically, which is what
//! makes the same crate usable on Android (NDK clang), iOS (Apple clang) and
//! the desktop toolchains.
//!
//! Environment variables understood by this script:
//!   CMAKE            cmake executable to use (default: `cmake` from PATH)
//!   PM3X_MINGW_BIN   directory containing gcc/g++ on Windows/MinGW
//!                    (default: first of C:\msys64\ucrt64\bin, ...)
//!   ANDROID_NDK_HOME Android NDK directory (also: ANDROID_NDK_ROOT,
//!                    ANDROID_NDK, ANDROID_NDK_LATEST_HOME)
//!   PM3X_CMAKE_GENERATOR  override the CMake generator
//!   PM3X_CMAKE_ARGS  extra arguments appended to the CMake configure line

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const COMMANDS: &[&str] = &["connect", "disconnect", "execute", "status", "version"];

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
        "cmake/CMakeLists.txt",
        "vendor/bzip2",
        "vendor/lz4",
        "proxmark3/client/src",
        "proxmark3/client/include",
        "proxmark3/client/deps",
        "proxmark3/common",
        "proxmark3/include",
        "proxmark3/common_fpga",
    ] {
        println!("cargo:rerun-if-changed={change}");
    }
    for var in [
        "CMAKE",
        "PM3X_MINGW_BIN",
        "PM3X_CMAKE_GENERATOR",
        "PM3X_CMAKE_ARGS",
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "ANDROID_NDK",
        "ANDROID_NDK_LATEST_HOME",
        "ANDROID_SDK_ROOT",
        "ANDROID_HOME",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    let proxmark3_dir = manifest_dir.join("proxmark3");
    if !proxmark3_dir
        .join("client/experimental_lib/CMakeLists.txt")
        .exists()
    {
        panic!(
            "\n\n\
             The Proxmark3 submodule is missing.\n\
             Fetch it once with:\n\n    \
             git submodule update --init --recursive\n\n\
             (expected sources in {})\n",
            proxmark3_dir.display()
        );
    }

    // ------------------------------------------------------------------
    // Toolchain selection
    // ------------------------------------------------------------------
    // CMake object paths inside rustc's target directory get long enough to
    // hit the 260 character Windows limit (a checkout below a deep directory
    // is enough).  Keep the CMake build tree in a short, per-OUT_DIR directory
    // under the system temp dir instead.
    let build_dir = cmake_build_dir(&out_dir);

    let mut configure = Command::new(cmake_program());
    configure
        .arg("-S")
        .arg(manifest_dir.join("cmake"))
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg("-DCMAKE_POSITION_INDEPENDENT_CODE=ON");

    if let Ok(extra) = env::var("PM3X_CMAKE_ARGS") {
        for arg in extra.split_whitespace() {
            configure.arg(arg);
        }
    }

    let mut path_prefix: Vec<PathBuf> = Vec::new();

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
            let mingw_root = mingw.parent().map(Path::to_path_buf);
            let gcc = gcc_path(&mingw);
            let gxx = gxx_path(&mingw);
            configure
                .arg(format!("-DCMAKE_C_COMPILER={}", gcc.display()))
                .arg(format!("-DCMAKE_CXX_COMPILER={}", gxx.display()));
            path_prefix.push(mingw.clone());
            if let Some(root) = mingw_root {
                let usr_bin = root.join("usr").join("bin");
                if usr_bin.exists() {
                    path_prefix.push(usr_bin); // provides sh.exe for mkversion.sh
                }
            }
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
                .arg(format!("-DANDROID_PLATFORM=android-{api}"));
        }
        "ios" => {
            let (sysroot, arch) = match target.as_str() {
                "aarch64-apple-ios" => ("iphoneos", "arm64"),
                "aarch64-apple-ios-sim" => ("iphonesimulator", "arm64"),
                "x86_64-apple-ios" => ("iphonesimulator", "x86_64"),
                other => panic!("unsupported iOS target: {other}"),
            };
            let deployment = env::var("PM3X_IOS_DEPLOYMENT_TARGET")
                .unwrap_or_else(|_| "13.0".to_string());
            configure
                .arg("-DCMAKE_SYSTEM_NAME=iOS")
                .arg(format!("-DCMAKE_OSX_SYSROOT={sysroot}"))
                .arg(format!("-DCMAKE_OSX_ARCHITECTURES={arch}"))
                .arg(format!("-DCMAKE_OSX_DEPLOYMENT_TARGET={deployment}"))
                .arg("-DCMAKE_C_COMPILER=clang")
                .arg("-DCMAKE_CXX_COMPILER=clang++");
        }
        _ => {}
    }

    if let Some(generator) = cmake_generator() {
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
    // C bridge
    //
    // Compiled before the client archives are declared: GNU ld resolves
    // archives left to right, so the object that calls into the client has to
    // come first.
    // ------------------------------------------------------------------
    let include_dirs = read_lines(&build_dir.join("pm3x_include_dirs.txt"));
    assert!(
        !include_dirs.is_empty(),
        "the Proxmark3 build did not report any include directory"
    );

    let mut bridge = cc::Build::new();
    bridge.file(manifest_dir.join("shim/pm3_bridge.c"));
    for dir in &include_dirs {
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
    // Link the produced archives
    // ------------------------------------------------------------------
    let archives = read_lines(&build_dir.join("pm3x_static_libs.txt"));
    assert!(
        !archives.is_empty(),
        "the Proxmark3 build did not report any static library"
    );

    for archive in &archives {
        let path = Path::new(archive);
        let dir = path
            .parent()
            .unwrap_or_else(|| panic!("invalid archive path: {archive}"));
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
    for archive in &archives {
        let name = archive_stem(Path::new(archive));
        println!("cargo:rustc-link-lib=static={name}");
    }

    // System libraries the client needs (zlib, sockets, frameworks, ...).
    let system_libs = read_lines(&build_dir.join("pm3x_system_libs.txt"));
    for lib in &system_libs {
        if let Some(framework) = lib.strip_prefix("framework:") {
            println!("cargo:rustc-link-lib=framework={framework}");
        } else {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    // Compile-time marker so `cargo build -vv` shows which target was built.
    println!("cargo:rustc-env=PM3X_TARGET_TRIPLE={target}");
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn cmake_program() -> String {
    env::var("CMAKE").unwrap_or_else(|_| "cmake".to_string())
}

fn cmake_generator() -> Option<String> {
    if let Ok(value) = env::var("PM3X_CMAKE_GENERATOR") {
        return Some(value);
    }
    // Ninja is faster and behaves the same on every host, prefer it when it is
    // installed.  Fall back to the platform default otherwise.
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

fn cmake_build_dir(out_dir: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    out_dir.hash(&mut hasher);
    let digest = format!("{:016x}", hasher.finish());

    #[cfg(windows)]
    {
        // Short base path, e.g. C:\Users\<user>\AppData\Local\Temp\pm3x-<hash>
        std::env::temp_dir().join(format!("pm3x-{digest}"))
    }
    #[cfg(not(windows))]
    {
        let _ = digest;
        out_dir.join("pm3build")
    }
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

fn read_lines(path: &Path) -> Vec<String> {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
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

fn run(command: &mut Command, what: &str) {
    let printable = format!("{command:?}");
    let status = command
        .status()
        .unwrap_or_else(|e| panic!("failed to {what} ({printable}): {e}"));
    if !status.success() {
        panic!("failed to {what} ({printable}): {status}");
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
        // Tolerate `PM3X_MINGW_BIN="C:\..."` written with quotes by a CI
        // script or a shell config.
        let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
        let dir = PathBuf::from(value);
        if has_gcc(&dir) {
            return Some(dir);
        }
    }
    let candidates = [
        r"C:\msys64\ucrt64\bin",
        r"C:\msys2\ucrt64\bin",
        r"C:\UserProgram\msys2\ucrt64\bin",
        r"C:\tools\msys64\ucrt64\bin",
    ];
    candidates
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
