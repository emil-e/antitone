cmake_minimum_required(VERSION 3.20)
project(antitone_dng LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
if(MSVC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
endif()
set(SDK "${CMAKE_CURRENT_SOURCE_DIR}/../../vendor/dng-sdk")
include(sources.cmake)

# Keep codec dependencies within the reviewed SDK archive, not system packages.
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
foreach(option TOOLS DEVTOOLS FUZZERS BENCHMARK EXAMPLES JNI JPEGLI DOXYGEN MANPAGES
               VIEWERS PLUGINS SJPEG OPENEXR TCMALLOC)
    set(JPEGXL_ENABLE_${option} OFF CACHE BOOL "" FORCE)
endforeach()
set(JPEGXL_ENABLE_SKCMS ON CACHE BOOL "" FORCE)
add_subdirectory("${SDK}/libjxl/libjxl" "${CMAKE_CURRENT_BINARY_DIR}/jxl" EXCLUDE_FROM_ALL)

add_library(xmp_core OBJECT ${XMP_CORE_SOURCES})
add_library(xmp_files OBJECT ${XMP_FILES_SOURCES})
if(APPLE)
    set(XMP_RESOURCE mac)
elseif(WIN32)
    set(XMP_RESOURCE win)
else()
    message(FATAL_ERROR "Antitone's DNG build integration supports macOS and Windows")
endif()
foreach(target xmp_core xmp_files)
    set_target_properties(${target} PROPERTIES CXX_STANDARD 11)
    target_compile_definitions(${target} PRIVATE
        AdobePrivate=1 ENABLE_CPP_DOM_MODEL=1 XML_STATIC=1 HAVE_EXPAT_CONFIG_H=1
        XMP_StaticBuild=1 XMP_64=1 XML_POOR_ENTROPY)
    if(APPLE)
        target_compile_options(${target} PRIVATE
            "$<$<COMPILE_LANGUAGE:CXX>:-funsigned-char;-fshort-enums;-fno-common>")
    elseif(MSVC)
        target_compile_options(${target} PRIVATE /J)
    endif()
    target_include_directories(${target} PRIVATE
        "${SDK}/xmp/toolkit"
        "${SDK}/xmp/toolkit/public/include"
        "${SDK}/xmp/toolkit/public/include/client-glue"
        "${SDK}/xmp/toolkit/XMPCore/third-party/expat/public/lib"
        "${SDK}/xmp/toolkit/XMPCore/resource/${XMP_RESOURCE}"
        "${SDK}/xmp/toolkit/XMPFiles/resource/${XMP_RESOURCE}"
        "${SDK}/xmp/toolkit/XMPFilesPlugins/api/source")
endforeach()
target_compile_definitions(xmp_core PRIVATE
    BUILDING_XMPCORE_LIB=1 XMP_COMPONENT_INT_NAMESPACE=AdobeXMPCore_Int
    BUILDING_XMPCORE_AS_STATIC=1)

add_library(dng STATIC ${DNG_SOURCES}
    $<TARGET_OBJECTS:xmp_core> $<TARGET_OBJECTS:xmp_files>)
file(STRINGS "${CMAKE_CURRENT_SOURCE_DIR}/defines.txt" DNG_DEFINES)
target_compile_definitions(dng PUBLIC ${DNG_DEFINES})
target_include_directories(dng PUBLIC
    "${SDK}/dng_sdk/source" "${SDK}/libjpeg"
    "${SDK}/xmp/toolkit/public/include")
target_link_libraries(dng PRIVATE jxl jxl_threads)

if(APPLE)
    foreach(target dng xmp_core xmp_files)
        target_compile_definitions(${target} PRIVATE
            MAC_ENV=1 Macintosh=1 qMacOS=1 qWinOS=0)
    endforeach()
    # The bundled zlib tests TARGET_OS_MAC for classic Mac OS, not Darwin.
    # Match Adobe's XMPFiles build instead of selecting that obsolete branch.
    target_compile_options(xmp_files PRIVATE -fno-define-target-os-macros)
    # Cargo consumes one archive, including the transitive static codec libraries.
    # This is library assembly, not harvesting objects from an executable build.
    set(ARCHIVE "${CMAKE_CURRENT_BINARY_DIR}/libantitone_dng_sdk.a")
    add_custom_command(OUTPUT "${ARCHIVE}"
        COMMAND /usr/bin/libtool -static -o "${ARCHIVE}"
            $<TARGET_FILE:dng> $<TARGET_FILE:jxl> $<TARGET_FILE:jxl_threads>
            $<TARGET_FILE:jxl_cms> $<TARGET_FILE:hwy>
            $<TARGET_FILE:brotlienc> $<TARGET_FILE:brotlidec> $<TARGET_FILE:brotlicommon>
        DEPENDS dng jxl jxl_threads jxl_cms hwy brotlienc brotlidec brotlicommon
        VERBATIM)
    add_custom_target(antitone_dng_sdk ALL DEPENDS "${ARCHIVE}")
elseif(WIN32)
    foreach(target dng xmp_core xmp_files)
        target_compile_definitions(${target} PRIVATE
            WIN_ENV=1 Macintosh=0 qMacOS=0 qWinOS=1 NOMINMAX UNICODE _UNICODE)
    endforeach()
    # Cargo consumes one archive, including the transitive static codec libraries.
    set(ARCHIVE "${CMAKE_CURRENT_BINARY_DIR}/antitone_dng_sdk.lib")
    add_custom_command(OUTPUT "${ARCHIVE}"
        COMMAND "${CMAKE_AR}" /NOLOGO /OUT:"${ARCHIVE}"
            $<TARGET_FILE:dng> $<TARGET_FILE:jxl> $<TARGET_FILE:jxl_threads>
            $<TARGET_FILE:jxl_cms> $<TARGET_FILE:hwy>
            $<TARGET_FILE:brotlienc> $<TARGET_FILE:brotlidec> $<TARGET_FILE:brotlicommon>
        DEPENDS dng jxl jxl_threads jxl_cms hwy brotlienc brotlidec brotlicommon
        VERBATIM)
    add_custom_target(antitone_dng_sdk ALL DEPENDS "${ARCHIVE}")
else()
    message(FATAL_ERROR "Antitone's DNG build integration supports macOS and Windows")
endif()
