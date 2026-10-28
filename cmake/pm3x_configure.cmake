#-----------------------------------------------------------------------------
# Injected into the upstream Proxmark3 CMake project through
#   -DCMAKE_PROJECT_INCLUDE=<this file>
#
# It runs at the end of the upstream project() call, i.e. before any target is
# created, so it can only adjust directory level settings.  Nothing here
# creates a project, a target or a library: the upstream build files stay
# untouched and keep owning the build.
#
# Two things the upstream build does not do on its own:
#   * Android's clang build does not enable AVX2/AVX-512 for
#     loclass/cipher_bs_avx*.c (the sources carry target pragmas everywhere
#     else, but upstream skips them for Android);
#   * iOS marks system() as unavailable, while client/src/cmdparser.c uses it
#     for the `!` shell escape (and Lua's os.execute() would too).
#-----------------------------------------------------------------------------

# AVX sources need their instruction sets; setting the property in this
# directory scope is enough because the upstream target is created here.
if(ANDROID AND CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64|i.86")
    set_source_files_properties(
        "${CMAKE_CURRENT_SOURCE_DIR}/../../client/src/loclass/cipher_bs_avx2.c"
        PROPERTIES COMPILE_OPTIONS -mavx2)
    if(CMAKE_SYSTEM_PROCESSOR MATCHES "x86_64|AMD64")
        set_source_files_properties(
            "${CMAKE_CURRENT_SOURCE_DIR}/../../client/src/loclass/cipher_bs_avx512.c"
            PROPERTIES COMPILE_OPTIONS -mavx512f)
    endif()
endif()

# iOS has no system(): route every call to an inline stub instead of patching
# the submodule.  The header includes <stdlib.h> first and then shadows the
# name, so the unavailable declaration is never referenced.
if(APPLE AND CMAKE_SYSTEM_NAME STREQUAL "iOS" AND PM3X_IOS_SYSTEM_HEADER)
    add_compile_options(-include "${PM3X_IOS_SYSTEM_HEADER}")
endif()

# Upstream builds with -Werror, which is right for the Proxmark3 project but
# not for a consumer build: the client is compiled by whatever host compiler
# the plugin user has.  The warning stays visible, the error promotion is
# dropped.  This has to run after the targets exist, hence the deferred call.
cmake_language(DEFER DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}" CALL pm3x_relax_warnings)

function(pm3x_relax_warnings)
    foreach(pm3x_target
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
            pm3rrg_rdv4_whereami)
        if(TARGET ${pm3x_target})
            target_compile_options(${pm3x_target} PRIVATE -Wno-error)
        endif()
    endforeach()
endfunction()
