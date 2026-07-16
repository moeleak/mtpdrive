#!/bin/sh
set -eu

# Force the complete host macOS SDK even when a Nix development shell has
# injected its own developer directory, split SDK, and library search paths.
unset DEVELOPER_DIR SDKROOT
unset NIX_CFLAGS_COMPILE NIX_CFLAGS_COMPILE_FOR_BUILD
unset NIX_LDFLAGS NIX_LDFLAGS_FOR_BUILD
SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path)"
export SDKROOT

exec /usr/bin/clang "$@"
