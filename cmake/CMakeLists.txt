#-----------------------------------------------------------------------------
# Wraps the upstream Proxmark3 client "experimental_lib" CMake build so that the
# Rust plugin can link the client as a static library.
#
# Responsibilities:
#   * provide the two compression libraries the client expects (bzip2, lz4)
#     from the vendored sources in vendor/, because they are rarely installed
#     on Windows/macOS/Android/iOS build hosts;
#   * disable every optional upstream feature that pulls in a GUI, Python,
#     readline or native Bluetooth;
#   * emit pm3x_static_libs.txt, the ordered list of archives the Rust build
#     has to link (Rust cannot reuse CMake's PRIVATE link interface of a static
#     library).
#
# Usage (driven by build.rs):
#   cmake -S cmake -B <out>/pm3build -G <generator> [toolchain options]
#   cmake --build <out>/pm3build --target pm3rrg_rdv4_static
#-----------------------------------------------------------------------------
cmake_minimum_required(VERSION 3.16)
project(pm3bridge LANGUAGES C CXX)

if(APPLE)
    # experimental_lib adds client/src/util_darwin.m on Apple platforms.
    enable_language(OBJC)
endif()

set(PM3X_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/..")
set(PM3_ROOT "${PM3X_ROOT}/proxmark3" CACHE PATH "Proxmark3 source tree")

if(NOT EXISTS "${PM3_ROOT}/client/experimental_lib/CMakeLists.txt")
    message(FATAL_ERROR
        "Proxmark3 sources not found in '${PM3_ROOT}'.\n"
        "Initialise the submodule first: git submodule update --init --recursive")
endif()

# Everything is linked into a Rust staticlib / cdylib, so position independent
# code is required for the Android and iOS targets.
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

#-----------------------------------------------------------------------------
# Vendored bzip2 + lz4 (used by client/src/cmdhfmfhard.c)
#-----------------------------------------------------------------------------
set(PM3X_VENDOR_LIB_DIR "${CMAKE_BINARY_DIR}/vendor-libs")

add_library(pm3x_bz2 STATIC
    "${PM3X_ROOT}/vendor/bzip2/blocksort.c"
    "${PM3X_ROOT}/vendor/bzip2/bzlib.c"
    "${PM3X_ROOT}/vendor/bzip2/compress.c"
    "${PM3X_ROOT}/vendor/bzip2/crctable.c"
    "${PM3X_ROOT}/vendor/bzip2/decompress.c"
    "${PM3X_ROOT}/vendor/bzip2/huffman.c"
    "${PM3X_ROOT}/vendor/bzip2/randtable.c"
)
target_include_directories(pm3x_bz2 PUBLIC "${PM3X_ROOT}/vendor/bzip2")
set_target_properties(pm3x_bz2 PROPERTIES
    OUTPUT_NAME bz2
    ARCHIVE_OUTPUT_DIRECTORY "${PM3X_VENDOR_LIB_DIR}"
)

add_library(pm3x_lz4 STATIC
    "${PM3X_ROOT}/vendor/lz4/lib/lz4.c"
    "${PM3X_ROOT}/vendor/lz4/lib/lz4frame.c"
    "${PM3X_ROOT}/vendor/lz4/lib/lz4hc.c"
    "${PM3X_ROOT}/vendor/lz4/lib/xxhash.c"
)
target_include_directories(pm3x_lz4 PUBLIC "${PM3X_ROOT}/vendor/lz4/lib")
set_target_properties(pm3x_lz4 PROPERTIES
    OUTPUT_NAME lz4
    ARCHIVE_OUTPUT_DIRECTORY "${PM3X_VENDOR_LIB_DIR}"
)

# Teach FindBZip2 / the LZ4 probes in experimental_lib about the vendored libs.
set(BZIP2_INCLUDE_DIR "${PM3X_ROOT}/vendor/bzip2" CACHE PATH "" FORCE)
set(BZIP2_LIBRARY_RELEASE "${PM3X_VENDOR_LIB_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}bz2${CMAKE_STATIC_LIBRARY_SUFFIX}" CACHE FILEPATH "" FORCE)
set(BZIP2_LIBRARY_DEBUG "${BZIP2_LIBRARY_RELEASE}" CACHE FILEPATH "" FORCE)
set(LZ4_INCLUDE_DIRS "${PM3X_ROOT}/vendor/lz4/lib" CACHE PATH "" FORCE)
set(LZ4_LIBRARIES "${PM3X_VENDOR_LIB_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}lz4${CMAKE_STATIC_LIBRARY_SUFFIX}" CACHE FILEPATH "" FORCE)

#-----------------------------------------------------------------------------
# Feature switches for the upstream client build
#-----------------------------------------------------------------------------
set(SKIPBT 1 CACHE STRING "No native BlueZ Bluetooth")
set(SKIPPYTHON 1 CACHE STRING "No embedded Python")
set(SKIPREADLINE 1 CACHE STRING "No readline")
set(SKIPLINENOISE 1 CACHE STRING "No linenoise (not vendored upstream)")
set(SKIPGD 1 CACHE STRING "No libgd (image export is not needed)")
set(SKIPJANSSONSYSTEM 1 CACHE STRING "Always use the bundled jansson")
set(SKIPWHEREAMISYSTEM 1 CACHE STRING "Always use the bundled whereami")
set(SKIPQT 1 CACHE STRING "No Qt GUI")
set(PM3X_PTHREAD 0 CACHE STRING "Link the client against pthread")

#-----------------------------------------------------------------------------
# Upstream client library
#-----------------------------------------------------------------------------
add_subdirectory("${PM3_ROOT}/client/experimental_lib" "${CMAKE_BINARY_DIR}/proxmark3")

#-----------------------------------------------------------------------------
# Toolchain compatibility
#
# Upstream builds with -Werror.  That is right for the Proxmark3 project but
# not for a consumer build: the client is compiled by whatever host compiler
# the plugin user has (gcc 12..16, clang 15..20, NDK clang, Apple clang) and
# new diagnostics in bundled third party code would break the crate for
# reasons unrelated to this plugin.  Keep the warnings visible, drop the
# error promotion for the upstream targets only.
#-----------------------------------------------------------------------------
set(PM3X_UPSTREAM_TARGETS
    pm3rrg_rdv4_static
    pm3rrg_rdv4_mbedtls
    pm3rrg_rdv4_cliparser
    pm3rrg_rdv4_lua
    pm3rrg_rdv4_tinycbor
    pm3rrg_rdv4_amiibo
    pm3rrg_rdv4_reveng
    pm3rrg_rdv4_hardnested
    pm3rrg_rdv4_id48
    pm3rrg_rdv4_mqtt
    pm3rrg_rdv4_jansson
    pm3rrg_rdv4_whereami
)
foreach(pm3x_target IN LISTS PM3X_UPSTREAM_TARGETS)
    if(TARGET ${pm3x_target})
        target_compile_options(${pm3x_target} PRIVATE -Wno-error)
    endif()
endforeach()

add_dependencies(pm3rrg_rdv4_static pm3x_bz2 pm3x_lz4)

get_target_property(PM3X_CLIENT_SOURCES pm3rrg_rdv4_static SOURCES)

# experimental_lib lists the AVX sources but, unlike client/CMakeLists.txt,
# never enables the instruction sets they need.  Mirror upstream's conditions.
# The properties are attached to the exact path strings the target registered,
# because CMake treats differently spelled paths as different sources.
foreach(pm3x_source IN LISTS PM3X_CLIENT_SOURCES)
    get_filename_component(pm3x_source_name "${pm3x_source}" NAME)
    if(pm3x_source_name STREQUAL "cipher_bs_avx2.c" AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64|i.86")
        set_source_files_properties("${pm3x_source}" PROPERTIES COMPILE_OPTIONS "-mavx2")
    elseif(pm3x_source_name STREQUAL "cipher_bs_avx512.c" AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
        set_source_files_properties("${pm3x_source}" PROPERTIES COMPILE_OPTIONS "-mavx512f")
    endif()
endforeach()

if(APPLE AND CMAKE_SYSTEM_NAME STREQUAL "iOS")
    # Lua's os.execute() must not call system(), which the iOS SDK marks as
    # unavailable.  Lua ships an LUA_USE_IOS switch for exactly this.
    target_compile_definitions(pm3rrg_rdv4_lua PRIVATE LUA_USE_IOS)

    # client/src/cmdparser.c implements the `!` shell escape with system(),
    # also unavailable on iOS.  Compile a build-local copy where the two call
    # sites go through pm3x_platform_system() (implemented in the bridge and
    # returning -1 on iOS) instead of touching the submodule.
    set(PM3X_CMDPARSER "${PM3_ROOT}/client/src/cmdparser.c")
    file(READ "${PM3X_CMDPARSER}" PM3X_CMDPARSER_SOURCE)
    string(REPLACE "ret = system(" "ret = pm3x_platform_system(" PM3X_CMDPARSER_SOURCE "${PM3X_CMDPARSER_SOURCE}")
    set(PM3X_CMDPARSER_PATCHED "${CMAKE_BINARY_DIR}/patched/cmdparser.c")
    file(WRITE "${PM3X_CMDPARSER_PATCHED}"
        "/* Generated by tauri-plugin-proxmark3: iOS has no system(). */\n"
        "int pm3x_platform_system(const char *command);\n"
        "${PM3X_CMDPARSER_SOURCE}")

    set(PM3X_CLIENT_SOURCES_PATCHED "")
    foreach(pm3x_source IN LISTS PM3X_CLIENT_SOURCES)
        get_filename_component(pm3x_source_name "${pm3x_source}" NAME)
        if(pm3x_source_name STREQUAL "cmdparser.c")
            list(APPEND PM3X_CLIENT_SOURCES_PATCHED "${PM3X_CMDPARSER_PATCHED}")
        else()
            list(APPEND PM3X_CLIENT_SOURCES_PATCHED "${pm3x_source}")
        endif()
    endforeach()
    set_property(TARGET pm3rrg_rdv4_static PROPERTY SOURCES ${PM3X_CLIENT_SOURCES_PATCHED})
endif()

#-----------------------------------------------------------------------------
# Emit the ordered archive list consumed by build.rs.
#-----------------------------------------------------------------------------
set(PM3X_LINK_TARGETS
    pm3rrg_rdv4_static
    pm3rrg_rdv4_mbedtls
    pm3rrg_rdv4_cliparser
    pm3rrg_rdv4_lua
    pm3rrg_rdv4_tinycbor
    pm3rrg_rdv4_amiibo
    pm3rrg_rdv4_reveng
    pm3rrg_rdv4_hardnested
    pm3rrg_rdv4_id48
    pm3rrg_rdv4_mqtt
    pm3rrg_rdv4_jansson
    pm3rrg_rdv4_whereami
    pm3x_bz2
    pm3x_lz4
)

set(PM3X_LINK_LINES "")
foreach(pm3x_target IN LISTS PM3X_LINK_TARGETS)
    if(NOT TARGET ${pm3x_target})
        message(FATAL_ERROR
            "The proxmark3 build no longer defines target '${pm3x_target}'.\n"
            "Update PM3X_LINK_TARGETS in cmake/CMakeLists.txt to match "
            "client/experimental_lib/CMakeLists.txt.")
    endif()
    get_target_property(pm3x_type ${pm3x_target} TYPE)
    if(pm3x_type STREQUAL "INTERFACE_LIBRARY")
        continue() # header only, nothing to link
    endif()
    string(APPEND PM3X_LINK_LINES "$<TARGET_FILE:${pm3x_target}>\n")
endforeach()

file(GENERATE OUTPUT "${CMAKE_BINARY_DIR}/pm3x_static_libs.txt" CONTENT "${PM3X_LINK_LINES}")

#-----------------------------------------------------------------------------
# Include directories and system libraries for the Rust build script.
#-----------------------------------------------------------------------------
# The bridge includes client headers that pull in the bundled dependencies
# (jansson, mbedtls, ...), so it needs the same include paths the client is
# compiled with.  Spelling them out is more reliable than reading target
# properties: the bundled deps record their include directories as paths
# relative to client/deps.
set(PM3X_INCLUDE_DIRS
    "${PM3_ROOT}/common"
    "${PM3_ROOT}/common_fpga"
    "${PM3_ROOT}/include"
    "${PM3_ROOT}/client/src"
    "${PM3_ROOT}/client/include"
    "${PM3_ROOT}/common/mbedtls"
    "${PM3_ROOT}/client/deps/jansson"
    "${PM3_ROOT}/client/deps/cliparser"
    "${PM3_ROOT}/client/deps/liblua"
    "${PM3_ROOT}/client/deps/tinycbor"
    "${PM3_ROOT}/client/deps/amiitool"
    "${PM3_ROOT}/client/deps/reveng"
    "${PM3_ROOT}/client/deps/hardnested"
    "${PM3_ROOT}/client/deps/id48/public"
    "${PM3_ROOT}/client/deps/mqtt"
    "${PM3_ROOT}/client/deps/vec"
    "${PM3_ROOT}/client/deps/whereami"
    "${PM3X_ROOT}/vendor/lz4/lib"
    "${PM3X_ROOT}/vendor/bzip2"
)

foreach(pm3x_target IN LISTS PM3X_LINK_TARGETS)
    if(NOT TARGET ${pm3x_target})
        continue()
    endif()
    get_target_property(pm3x_dirs ${pm3x_target} INCLUDE_DIRECTORIES)
    if(pm3x_dirs)
        foreach(pm3x_dir IN LISTS pm3x_dirs)
            # Absolute, existing directories only (generator expressions and
            # relative paths are handled by the list above).
            if(NOT pm3x_dir MATCHES "\\$<" AND IS_ABSOLUTE "${pm3x_dir}" AND IS_DIRECTORY "${pm3x_dir}")
                list(APPEND PM3X_INCLUDE_DIRS "${pm3x_dir}")
            endif()
        endforeach()
    endif()
endforeach()

list(REMOVE_DUPLICATES PM3X_INCLUDE_DIRS)

string(REPLACE ";" "\n" PM3X_INCLUDE_DIRS_TEXT "${PM3X_INCLUDE_DIRS}")
file(WRITE "${CMAKE_BINARY_DIR}/pm3x_include_dirs.txt" "${PM3X_INCLUDE_DIRS_TEXT}\n")

set(PM3X_SYSTEM_LIBS "")
# ZLIB_FOUND is set inside the proxmark3 subdirectory, so probe again here.
find_package(ZLIB QUIET)
if(ZLIB_FOUND)
    list(APPEND PM3X_SYSTEM_LIBS z)
endif()
if(WIN32)
    list(APPEND PM3X_SYSTEM_LIBS ws2_32 pthread)
endif()
if(UNIX AND NOT APPLE AND NOT ANDROID)
    list(APPEND PM3X_SYSTEM_LIBS m pthread)
endif()
if(APPLE)
    list(APPEND PM3X_SYSTEM_LIBS "framework:Foundation")
    find_library(PM3X_UIKIT_LIBRARY UIKit)
    if(PM3X_UIKIT_LIBRARY)
        list(APPEND PM3X_SYSTEM_LIBS "framework:UIKit")
    else()
        find_library(PM3X_APPKIT_LIBRARY AppKit)
        if(PM3X_APPKIT_LIBRARY)
            list(APPEND PM3X_SYSTEM_LIBS "framework:AppKit")
        endif()
    endif()
endif()

string(REPLACE ";" "\n" PM3X_SYSTEM_LIBS_TEXT "${PM3X_SYSTEM_LIBS}")
file(WRITE "${CMAKE_BINARY_DIR}/pm3x_system_libs.txt" "${PM3X_SYSTEM_LIBS_TEXT}\n")
