#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "Branding asset generation requires macOS CoreText, ImageIO and iconutil." >&2
    exit 1
fi
mkdir -p .cache/branding-assets/swift-module-cache
swift -module-cache-path .cache/branding-assets/swift-module-cache native/branding_assets.swift
