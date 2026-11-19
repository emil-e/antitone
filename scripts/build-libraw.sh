#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
cache=.cache/native-deps
archive="$cache/LibRaw-0.22.3.tar.gz"
upstream=$(awk -F= '$1 == "upstream" {print $2}' native/libraw/dht-hashes.txt)
patched=$(awk -F= '$1 == "patched" {print $2}' native/libraw/dht-hashes.txt)
source="$cache/LibRaw-0.22.3-$(printf '%.12s' "$patched")-rawspeed-openmp"
omp_version=$(awk -F= '$1 == "version" {print $2}' native/openmp/pins.txt)
rs_revision=$(awk -F= '$1 == "revision" {print $2}' native/rawspeed/pins.txt)
openmp="$PWD/$cache/openmp-$omp_version-install"
rawspeed="$PWD/$cache/rawspeed-$(printf '%.12s' "$rs_revision")-macos-arm64"
mkdir -p "$cache"
./scripts/build-rawspeed.sh

if [ ! -f "$archive" ]; then
    if [ -f .cache/deps/LibRaw-0.22.3.tar.gz ]; then
        cp .cache/deps/LibRaw-0.22.3.tar.gz "$archive"
    else
        curl --fail --location https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.3 -o "$archive.tmp"
        mv "$archive.tmp" "$archive"
    fi
fi
printf '%s  %s\n' 7edf96c5b70cff88980a02ab0fc8af125cb6a4a377985573c654439d9d02355c "$archive" | shasum -a 256 -c -
if [ ! -d "$source" ]; then
    mkdir -p "$source"
    tar -xf "$archive" -C "$source" --strip-components=1
fi

# Apply only to the verified pinned source, including on reuse of an existing build.
actual=$(shasum -a 256 "$source/src/demosaic/dht_demosaic.cpp" | awk '{print $1}')
if [ "$actual" = "$upstream" ]; then
    git -c core.autocrlf=false -c core.eol=lf apply --directory="$source" native/libraw/dht.patch
elif [ "$actual" != "$patched" ]; then
    printf '%s\n' 'LibRaw DHT source differs from the pinned upstream and patched revisions.' >&2
    exit 1
fi
printf '%s  %s\n' "$patched" "$source/src/demosaic/dht_demosaic.cpp" | shasum -a 256 -c -

make -C "$source" -f Makefile.devel -j4 CC=/usr/bin/clang CXX=/usr/bin/clang++ \
    CFLAGS="-DLIBRAW_USE_AUTOPTR -DUSE_ZLIB -g -I. -pedantic -Wno-long-long -Wno-overflow -O3 -Wall -mmacosx-version-min=14.0 -Xclang -fopenmp -I$openmp/include -DUSE_RAWSPEED3 -I$PWD/native/rawspeed" \
    lib/libraw_r.a

/usr/bin/clang++ -std=c++20 -O2 -mmacosx-version-min=14.0 -Xclang -fopenmp \
    -I"$openmp/include" -I"$rawspeed/include" -I"$rawspeed/include/rawspeed" \
    -c native/rawspeed/adapter.cpp -o "$source/lib/rawspeed_adapter.o"
for check in dht_safety rawspeed_adoption; do
    if [ "$check" = dht_safety ]; then
        test_source=native/libraw/dht_safety.cpp
    else
        test_source=native/rawspeed/adoption_tests.cpp
    fi
    /usr/bin/clang++ -std=c++17 -O1 -g -mmacosx-version-min=14.0 \
        -Xclang -fopenmp -I"$openmp/include" -I"$source" -Inative/rawspeed \
        "$test_source" "$source/lib/rawspeed_adapter.o" "$source/lib/libraw_r.a" \
        "$rawspeed/lib/librawspeed.a" "$rawspeed/lib/libpugixml.a" \
        -L"$openmp/lib" -Wl,-rpath,"$openmp/lib" -lomp -lz \
        -o "$source/lib/$check"
    "$source/lib/$check" "$PWD/tests/fixtures/raw/patches.dng"
done
printf 'rawspeed3;llvm-openmp-%s\n' "$omp_version" > "$source/antitone-features.txt"
