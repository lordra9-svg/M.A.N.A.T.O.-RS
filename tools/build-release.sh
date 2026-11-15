#!/usr/bin/env bash
# Builds the release binaries into `dist/`.
#
# Linux (dynamic, recommended): uses the host glibc toolchain.
# Linux (static musl): fully static, but wgpu's Vulkan loader is dlopened and
# host Vulkan drivers are glibc shared objects. Use the dynamic Linux build
# for rendering; the musl build is provided for static/portable deployments
# that supply a musl-compatible graphics stack.
# Windows (x86_64-pc-windows-gnu): needs `x86_64-w64-mingw32-gcc` on PATH
# (Homebrew: `brew install mingw-w64`).
set -euo pipefail

cd "$(dirname "$0")/.."

DIST="${DIST:-dist}"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
: "${PKG_CONFIG_PATH:=/home/linuxbrew/.linuxbrew/Cellar/systemd/262.2/lib/pkgconfig}"
export PKG_CONFIG_PATH

rustup target add x86_64-pc-windows-gnu x86_64-unknown-linux-musl >/dev/null

echo "==> Linux (glibc, dynamic)"
cargo build --release -p manato-app
echo "==> Linux (musl, static)"
cargo build --release -p manato-app --target x86_64-unknown-linux-musl --no-default-features
echo "==> Windows (x86_64-pc-windows-gnu)"
cargo build --release -p manato-app --target x86_64-pc-windows-gnu

mkdir -p "$DIST"
cp target/release/manato "$DIST/manato-linux-x86_64"
cp target/x86_64-unknown-linux-musl/release/manato "$DIST/manato-linux-x86_64-musl"
cp target/x86_64-pc-windows-gnu/release/manato.exe "$DIST/manato-windows-x86_64.exe"
cp LICENSE README.md "$DIST/"

echo
ls -lh "$DIST"
echo
file "$DIST"/manato-* | sed 's/^/  /'
