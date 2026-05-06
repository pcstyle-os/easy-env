#!/usr/bin/env bash
set -euo pipefail

REPO="${EASYENV_REPO:-pcstyle/easy-env}"
VERSION="${1:-latest}"
INSTALL_DIR="${EASYENV_INSTALL_DIR:-/usr/local/bin}"

os=$(uname -s)
arch=$(uname -m)

case "$os" in
  Darwin) os_slug="macos" ;;
  Linux) os_slug="linux" ;;
  *)
    echo "Unsupported OS: $os" >&2
    exit 1
    ;;
esac

case "$arch" in
  arm64|aarch64) arch_slug="aarch64" ;;
  x86_64|amd64) arch_slug="x86_64" ;;
  *)
    echo "Unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

asset="easyenv-${os_slug}-${arch_slug}.tar.gz"
if [[ "$VERSION" == "latest" ]]; then
  url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
  url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL "$url" -o "$tmpdir/$asset"
tar -xzf "$tmpdir/$asset" -C "$tmpdir"
mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmpdir/easyenv-${os_slug}-${arch_slug}/easyenv" "$INSTALL_DIR/easyenv"

echo "Installed easyenv to $INSTALL_DIR/easyenv"
easyenv --version
