#!/usr/bin/env sh
# JavaR CLI installer — curl | sh style
# Author: Roberto de Souza <rabbittrix@hotmail.com>
set -eu

REPO="${JAVAR_REPO:-https://github.com/rabbittrix/javar}"
VERSION="${JAVAR_VERSION:-latest}"
PREFIX="${JAVAR_PREFIX:-$HOME/.javar}"
BIN_DIR="${JAVAR_BIN_DIR:-$PREFIX/bin}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;;
esac

case "$OS" in
  linux|darwin) ;;
  mingw*|msys*|cygwin*)
    echo "On Windows, use scripts/install.ps1 or cargo install." >&2
    exit 1
    ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

echo "==> Installing JavaR ($VERSION) for $OS-$ARCH"
mkdir -p "$BIN_DIR" "$PREFIX"

if command -v cargo >/dev/null 2>&1 && [ "${JAVAR_FROM_SOURCE:-0}" = "1" ]; then
  echo "==> Building from source via cargo"
  cargo install --git "$REPO" --locked javar-cli --root "$PREFIX"
else
  # Placeholder release URL — replace when binaries are published.
  TARBALL="javar-${VERSION}-${OS}-${ARCH}.tar.gz"
  URL="${REPO}/releases/download/${VERSION}/${TARBALL}"
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" -o "$TMP/$TARBALL" || {
      echo "Release asset not found; cloning and building with cargo..." >&2
      if ! command -v cargo >/dev/null 2>&1; then
        echo "Install Rust from https://rustup.rs and re-run." >&2
        exit 1
      fi
      cargo install --git "$REPO" javar-cli --root "$PREFIX"
      echo "==> Installed javar to $BIN_DIR"
      echo "Add to PATH: export PATH=\"$BIN_DIR:\$PATH\""
      exit 0
    }
  else
    echo "curl required" >&2
    exit 1
  fi
  tar -xzf "$TMP/$TARBALL" -C "$PREFIX"
fi

# Convenience symlink
if [ -x "$PREFIX/bin/javar" ]; then
  :
elif [ -x "$BIN_DIR/javar" ]; then
  :
fi

echo "==> Installed javar to $BIN_DIR"
echo "Add to PATH:"
echo "  export PATH=\"$BIN_DIR:\$PATH\""
echo
echo "Then run:"
echo "  javar init"
echo "  javar run"
echo "  javar status"
