#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cache=.cache/native-deps
version=22.1.8
archive="$cache/llvm-project-$version.src.tar.xz"
source="$cache/llvm-project-$version.src"
prefix="$PWD/$cache/openmp-$version-install"
mkdir -p "$cache"
if [ ! -f "$archive" ]; then
    curl --fail --location "https://github.com/llvm/llvm-project/releases/download/llvmorg-$version/llvm-project-$version.src.tar.xz" -o "$archive.tmp"
    mv "$archive.tmp" "$archive"
fi
printf '%s  %s\n' 922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888 "$archive" | shasum -a 256 -c -
if [ ! -d "$source/openmp" ]; then
    tar -xf "$archive" -C "$cache" \
        "llvm-project-$version.src/openmp" "llvm-project-$version.src/cmake" \
        "llvm-project-$version.src/llvm/cmake" "llvm-project-$version.src/LICENSE.TXT"
fi
cmake -S "$source/openmp" -B "$cache/openmp-$version-build" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=/usr/bin/clang \
    -DCMAKE_CXX_COMPILER=/usr/bin/clang++ -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DCMAKE_INSTALL_PREFIX="$prefix" \
    -DCMAKE_INSTALL_NAME_DIR=@rpath -DCMAKE_MACOSX_RPATH=ON \
    -DLIBOMP_ENABLE_SHARED=ON -DLIBOMP_USE_STDCPPLIB=OFF \
    -DLIBOMP_INSTALL_ALIASES=OFF -DOPENMP_ENABLE_LIBOMPTARGET=OFF
cmake --build "$cache/openmp-$version-build" --parallel 4
cmake --install "$cache/openmp-$version-build"
mkdir -p "$prefix/share/licenses"
cp "$source/openmp/LICENSE.TXT" "$prefix/share/licenses/LLVM-OpenMP.txt"
