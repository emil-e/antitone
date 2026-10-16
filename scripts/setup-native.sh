#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'Native editor setup currently supports macOS only.' >&2
    exit 1
fi
for tool in curl shasum tar make cmake ninja xcrun; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Missing prerequisite: %s. See README.md.\n' "$tool" >&2
        exit 1
    fi
done
if ! xcrun --find clang >/dev/null 2>&1 || ! xcrun --show-sdk-path >/dev/null 2>&1; then
    printf '%s\n' 'Select an installed Xcode or command-line tools developer directory. See README.md.' >&2
    exit 1
fi

./scripts/build-libraw.sh
./scripts/build-ocio.sh
