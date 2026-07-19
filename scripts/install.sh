#!/usr/bin/env sh
# JavaR one-liner installer (Linux / macOS)
# Author: Roberto de Souza <rabbittrix@hotmail.com>
#
# Usage:
#   curl -fsSL https://javar.dev/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh | sh
#
# Env:
#   JAVAR_REPO   GitHub org/repo (default: rabbittrix/JavaR)

set -eu

REPO="${JAVAR_REPO:-rabbittrix/JavaR}"
INSTALL_DIR="${HOME}/.javar/bin"
USER_AGENT="javar-install"

banner() {
  printf '\n'
  printf '  JavaR installer\n'
  printf '  by Roberto de Souza\n'
  printf '\n'
}

step() { printf '· %s\n' "$*"; }
ok()   { printf '✓ %s\n' "$*"; }
warn() { printf '! %s\n' "$*"; }
die()  { printf '! %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

detect_archive() {
  OS="$(uname -s)"
  ARCH="$(uname -m)"
  case "$OS" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *) die "Unsupported OS: $OS" ;;
  esac
  case "$ARCH" in
    x86_64|amd64) ARCH_TAG="x86_64" ;;
    arm64|aarch64) ARCH_TAG="aarch64" ;;
    *) die "Unsupported arch: $ARCH" ;;
  esac
  ARCHIVE="javar-${PLATFORM}-${ARCH_TAG}"
}

download_release() {
  need_cmd curl
  need_cmd unzip
  need_cmd find
  need_cmd install

  api="https://api.github.com/repos/${REPO}/releases/latest"
  step "Querying $api"
  json="$(curl -fsSL -H "User-Agent: ${USER_AGENT}" "$api")" || return 1

  url="$(printf '%s' "$json" \
    | sed -n "s/.*\"browser_download_url\": \"\\([^\"]*${ARCHIVE}[^\"]*\\.zip\\)\".*/\\1/p" \
    | head -n1)"
  if [ -z "$url" ]; then
    warn "No ${ARCHIVE}.zip asset on the latest release"
    return 1
  fi

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT INT TERM
  step "Downloading $url"
  curl -fsSL "$url" -o "$tmp/javar.zip"
  mkdir -p "$tmp/out"
  unzip -q "$tmp/javar.zip" -d "$tmp/out"

  bin="$(find "$tmp/out" -type f -name javar | head -n1)"
  [ -n "$bin" ] || die "javar binary missing in archive"
  install -m 755 "$bin" "$INSTALL_DIR/javar"

  lib="$(find "$tmp/out" -type f \( -name 'libjavar_core.so' -o -name 'libjavar_core.dylib' \) | head -n1 || true)"
  if [ -n "${lib:-}" ]; then
    cp "$lib" "$INSTALL_DIR/"
  fi

  jar="$(find "$tmp/out" -type f -name 'javar-agent*.jar' ! -name '*sources*' ! -name '*javadoc*' ! -name '*original*' | head -n1 || true)"
  if [ -n "${jar:-}" ]; then
    cp "$jar" "$INSTALL_DIR/javar-agent.jar"
  fi

  trap - EXIT INT TERM
  rm -rf "$tmp"
  ok "Installed release bits to $INSTALL_DIR"
  return 0
}

build_from_source() {
  need_cmd git
  need_cmd cargo

  step "Building from source via cargo…"
  src="$(mktemp -d)"
  trap 'rm -rf "$src"' EXIT INT TERM
  git clone --depth 1 "https://github.com/${REPO}.git" "$src"
  cd "$src/javar-project"

  if command -v mvn >/dev/null 2>&1; then
    step "Packaging javar-agent (Maven)"
    (cd javar-agent && mvn -q -DskipTests package) || warn "Maven package failed — CLI may ship without embedded agent"
  else
    warn "Maven not found — agent will not be embedded in this build"
  fi

  step "cargo build --release -p javar-core"
  cargo build --release -p javar-core
  step "cargo build --release -p javar-cli"
  cargo build --release -p javar-cli

  install -m 755 target/release/javar "$INSTALL_DIR/javar"
  for lib in target/release/libjavar_core.so target/release/libjavar_core.dylib; do
    [ -f "$lib" ] && cp "$lib" "$INSTALL_DIR/"
  done
  if [ -d javar-agent/target ]; then
    jar="$(find javar-agent/target -maxdepth 1 -type f -name 'javar-agent*.jar' ! -name '*sources*' ! -name '*javadoc*' ! -name '*original*' | head -n1 || true)"
    if [ -n "${jar:-}" ]; then
      cp "$jar" "$INSTALL_DIR/javar-agent.jar"
    fi
  fi

  trap - EXIT INT TERM
  rm -rf "$src"
  ok "Built and installed to $INSTALL_DIR"
}

# --- main ---
banner
mkdir -p "$INSTALL_DIR"
detect_archive

if ! download_release; then
  step "Falling back to source build"
  build_from_source
fi

[ -x "$INSTALL_DIR/javar" ] || die "Install failed: $INSTALL_DIR/javar missing"

step "Running javar setup"
"$INSTALL_DIR/javar" setup || warn "javar setup returned a non-zero exit code"

printf '\n'
ok "Done. Open a new terminal and run:  javar run"
printf '  Install dir: %s\n\n' "$INSTALL_DIR"
