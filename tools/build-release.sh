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

# mlua vendors Lua, so the musl target needs a musl C toolchain. Prefer a
# musl-cross toolchain (installed under ~/.local/share/manato) and fall back to
# Debian's musl-gcc wrapper.
MUSL_BIN="${MUSL_BIN:-$HOME/.local/share/manato/x86_64-linux-musl-cross/bin}"
if [[ -x "$MUSL_BIN/x86_64-linux-musl-gcc" ]]; then
  export PATH="$MUSL_BIN:$PATH"
  export CC_x86_64_unknown_linux_musl="$MUSL_BIN/x86_64-linux-musl-gcc"
elif command -v musl-gcc >/dev/null 2>&1; then
  export CC_x86_64_unknown_linux_musl=musl-gcc
else
  echo "error: musl C toolchain not found (set MUSL_BIN or install musl-tools)" >&2
  exit 1
fi
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$CC_x86_64_unknown_linux_musl"

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
