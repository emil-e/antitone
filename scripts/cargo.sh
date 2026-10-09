#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export RUSTUP_HOME="$PWD/.cache/rustup"
version=1.99.0
if ! rustup run "$version" rustc --version >/dev/null 2>&1; then
    rustup set auto-self-update disable
    rustup toolchain install "$version" --profile minimal --component rustfmt,clippy
fi
exec rustup run "$version" cargo "$@"
