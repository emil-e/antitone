cmake_minimum_required(VERSION 3.22)
project(antitone_rawspeed LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

foreach(input RAWSPEED_SOURCE PUGIXML_SOURCE RAWSPEED_REVISION)
  if(NOT DEFINED ${input})
    message(FATAL_ERROR "${input} must identify the pinned native dependency")
  endif()
endforeach()

# Only libraries used by the unpack adapter. JPEG here means lossy DNG JPEG,
# not the built-in lossless-JPEG implementation used by camera RAWs.
set(BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(BUILD_BENCHMARKING OFF CACHE BOOL "" FORCE)
set(BUILD_FUZZERS OFF CACHE BOOL "" FORCE)
set(BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(BUILD_DOCS OFF CACHE BOOL "" FORCE)
set(USE_XMLLINT OFF CACHE BOOL "" FORCE)
set(WITH_JPEG OFF CACHE BOOL "" FORCE)
set(WITH_ZLIB OFF CACHE BOOL "" FORCE)
set(WITH_OPENMP ON CACHE BOOL "" FORCE)
set(USE_BUNDLED_PUGIXML ON CACHE BOOL "" FORCE)
set(PUGIXML_PATH "${PUGIXML_SOURCE}" CACHE PATH "" FORCE)
# Distribution builds must not require the builder's instruction set.
set(RAWSPEED_MARCH "" CACHE INTERNAL "" FORCE)
add_subdirectory("${RAWSPEED_SOURCE}" rawspeed)

file(READ "${RAWSPEED_SOURCE}/data/cameras.xml" CAMERA_XML)
configure_file(camera_db.h.in antitone_rawspeed_camera_db.h @ONLY)

add_executable(adapter_tests adapter.cpp adapter_tests.cpp)
target_include_directories(adapter_tests PRIVATE
  "${CMAKE_CURRENT_BINARY_DIR}" "${RAWSPEED_SOURCE}/src/librawspeed"
  "${RAWSPEED_SOURCE}/src/external" "${CMAKE_CURRENT_BINARY_DIR}/rawspeed/src")
target_link_libraries(adapter_tests PRIVATE rawspeed pugixml)

install(TARGETS rawspeed pugixml ARCHIVE DESTINATION lib)
install(DIRECTORY "${RAWSPEED_SOURCE}/src/librawspeed/" DESTINATION include/rawspeed
  FILES_MATCHING PATTERN "*.h")
install(DIRECTORY "${RAWSPEED_SOURCE}/src/external/" DESTINATION include/rawspeed
  FILES_MATCHING PATTERN "*.h")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/rawspeed/src/rawspeedconfig.h" DESTINATION include/rawspeed)
install(FILES "${PUGIXML_SOURCE}/src/pugixml.hpp" "${PUGIXML_SOURCE}/src/pugiconfig.hpp"
  "${CMAKE_CURRENT_BINARY_DIR}/antitone_rawspeed_camera_db.h" DESTINATION include)
install(FILES "${RAWSPEED_SOURCE}/LICENSE" DESTINATION share/licenses RENAME RawSpeed-LGPL.txt)
install(FILES "${RAWSPEED_SOURCE}/credits.txt" DESTINATION share/licenses RENAME RawSpeed-credits.txt)
install(FILES "${RAWSPEED_SOURCE}/data/cameras.xml" DESTINATION share/licenses)
install(FILES "${PUGIXML_SOURCE}/LICENSE.md" DESTINATION share/licenses RENAME pugixml.txt)
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/compiler.txt" "${CMAKE_CXX_COMPILER}\n")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/compiler.txt" DESTINATION share/antitone)
