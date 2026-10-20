#!/bin/sh
# Installs grove from a GitHub release:
#   curl -fsSL https://github.com/ajongaro/grove/releases/latest/download/install.sh | sh
# Picks the binary for this OS and CPU, checks it against the release's
# SHA256SUMS, and puts it in ~/.local/bin. Re-run it to upgrade.
#   GROVE_VERSION=v0.2.0      a specific release instead of the latest
#   GROVE_INSTALL_DIR=<dir>   somewhere other than ~/.local/bin
set -eu

repo="https://github.com/ajongaro/grove"
version="${GROVE_VERSION:-latest}"
dir="${GROVE_INSTALL_DIR:-$HOME/.local/bin}"
if [ -n "${GROVE_DOWNLOAD_URL:-}" ]; then
  base="$GROVE_DOWNLOAD_URL"
elif [ "$version" = latest ]; then
  base="$repo/releases/latest/download"
else
  base="$repo/releases/download/$version"
fi

fail() { echo "grove install: $*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
  *) fail "$(uname -s) isn't supported (macOS and Linux only)" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) target="aarch64-$os" ;;
  x86_64 | amd64) target="x86_64-$os" ;;
  # 32-bit ARM Linux (Pi Zero, older Pis): one armv6 build runs on both.
  armv6l | armv7l)
    [ "$os" = unknown-linux-musl ] || fail "$(uname -m) is supported on Linux only"
    target=arm-unknown-linux-musleabihf ;;
  *) fail "$(uname -m) CPUs aren't supported (arm64, x86_64 and 32-bit ARM Linux only)" ;;
esac
asset="grove-$target.tar.gz"

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  fail "needs sha256sum or shasum to check the download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $asset ($version)"
curl -fsSL "$base/$asset" -o "$tmp/$asset" || fail "couldn't download $base/$asset"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "couldn't download $base/SHA256SUMS"
want="$(awk -v a="$asset" '$2 == a || $2 == "*" a { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$want" ] || fail "SHA256SUMS has no entry for $asset"
[ "$(sha256 "$tmp/$asset")" = "$want" ] || fail "checksum mismatch for $asset; nothing installed"

tar -xzf "$tmp/$asset" -C "$tmp" grove
mkdir -p "$dir"
# Replace by rename, so a running grove keeps its old file.
mv -f "$tmp/grove" "$dir/grove.new"
chmod 755 "$dir/grove.new"
mv -f "$dir/grove.new" "$dir/grove"
echo "Installed $("$dir/grove" --version 2>/dev/null || echo grove) to $dir/grove"

case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "Add $dir to your PATH, for example:  echo 'export PATH=\"$dir:\$PATH\"' >> ~/.zshrc" ;;
esac
command -v tmux >/dev/null 2>&1 ||
  echo "Grove needs tmux 3.2 or newer: brew install tmux (macOS) or sudo apt install tmux (Debian/Ubuntu)"
echo "Then run: grove doctor"
