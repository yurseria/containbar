#!/usr/bin/env bash
# Bundle Colima + Lima binaries into src-tauri/runtime/
# Run before `tauri build` to include runtime in the app bundle.
set -euo pipefail

mkdir -p "$(dirname "$0")/../src-tauri/runtime"
RUNTIME_DIR="$(cd "$(dirname "$0")/../src-tauri/runtime" && pwd)"
ARCH="$(uname -m)"

info()  { printf '\033[1;34m%s\033[0m\n' "$*"; }
error() { printf '\033[1;31mError: %s\033[0m\n' "$*" >&2; exit 1; }

# Check dependencies
command -v colima >/dev/null 2>&1 || error "colima not found. Install with: brew install colima"
command -v limactl >/dev/null 2>&1 || error "limactl not found. Should be installed with colima."
command -v docker-credential-osxkeychain >/dev/null 2>&1 || error "docker-credential-osxkeychain not found. Install with: brew install docker-credential-helper"

COLIMA_BIN="$(readlink -f "$(which colima)")"
LIMACTL_BIN="$(readlink -f "$(which limactl)")"
LIMA_DIR="$(dirname "$(dirname "$LIMACTL_BIN")")"

# Find standalone docker CLI (not OrbStack's)
DOCKER_BIN="$(readlink -f /opt/homebrew/bin/docker 2>/dev/null || readlink -f "$(which docker)" 2>/dev/null)"
[ -z "$DOCKER_BIN" ] && error "docker CLI not found. Install with: brew install docker"

info "Bundling Colima runtime..."
info "  colima:  $COLIMA_BIN"
info "  limactl: $LIMACTL_BIN"
info "  lima:    $LIMA_DIR"
info "  docker:  $DOCKER_BIN"

# Clean and copy
rm -rf "$RUNTIME_DIR/colima" "$RUNTIME_DIR/lima" "$RUNTIME_DIR/docker"
mkdir -p "$RUNTIME_DIR/colima/bin" "$RUNTIME_DIR/lima/bin" "$RUNTIME_DIR/lima/share/lima" "$RUNTIME_DIR/lima/libexec" "$RUNTIME_DIR/docker/bin"

# Colima binary
cp "$COLIMA_BIN" "$RUNTIME_DIR/colima/bin/colima"

# Lima binaries and data
cp -R "$LIMA_DIR/bin/"* "$RUNTIME_DIR/lima/bin/"
cp -R "$LIMA_DIR/libexec/"* "$RUNTIME_DIR/lima/libexec/"
cp -R "$LIMA_DIR/share/lima/"* "$RUNTIME_DIR/lima/share/lima/"

# Docker CLI
cp "$DOCKER_BIN" "$RUNTIME_DIR/docker/bin/docker"
cp "$(command -v docker-credential-osxkeychain)" "$RUNTIME_DIR/docker/bin/docker-credential-osxkeychain"

# Ensure read+write+execute
chmod 755 "$RUNTIME_DIR/colima/bin/colima"
chmod 755 "$RUNTIME_DIR/lima/bin/"*
chmod 755 "$RUNTIME_DIR/lima/libexec/"*
chmod 755 "$RUNTIME_DIR/docker/bin/docker"
chmod 755 "$RUNTIME_DIR/docker/bin/docker-credential-osxkeychain"

TOTAL="$(du -sh "$RUNTIME_DIR" | awk '{print $1}')"
info "Done! Runtime bundled to $RUNTIME_DIR ($TOTAL)"
