#!/usr/bin/env bash
# Prints the Homebrew formula for a release, from its SHA256SUMS:
#   scripts/formula.sh 0.2.0 dist/SHA256SUMS > Formula/grove.rb
# The formula installs the prebuilt binaries the release workflow made.
# GROVE_RELEASE_URL points it at other copies, for testing.
set -euo pipefail
version="$1"
sums="$2"
url="${GROVE_RELEASE_URL:-https://github.com/ajongaro/grove/releases/download/v$version}"

# asset <target> <var>: sets <var> to that binary's url and sha256 lines.
# No $(...) around the check, so a missing checksum stops the script.
asset() {
  local s
  s="$(awk -v a="grove-$1.tar.gz" '$2 == a || $2 == "*" a { print $1 }' "$sums")"
  [[ -n "$s" ]] || { echo "formula.sh: no checksum for $1 in $sums" >&2; exit 1; }
  printf -v "$2" '      url "%s/grove-%s.tar.gz"\n      sha256 "%s"' "$url" "$1" "$s"
}
asset aarch64-apple-darwin mac_arm
asset x86_64-apple-darwin mac_intel
asset aarch64-unknown-linux-musl linux_arm
asset x86_64-unknown-linux-musl linux_intel

cat <<EOF
# Written by scripts/formula.sh in ajongaro/grove; don't edit by hand.
class Grove < Formula
  desc "Dashboard for Claude Code sessions, in tmux"
  homepage "https://github.com/ajongaro/grove"
  version "$version"
  license "MIT"

  depends_on "tmux"

  on_macos do
    on_arm do
$mac_arm
    end
    on_intel do
$mac_intel
    end
  end

  on_linux do
    on_arm do
$linux_arm
    end
    on_intel do
$linux_intel
    end
  end

  def install
    bin.install "grove"
  end

  def caveats
    <<~EOS
      Grove needs Claude Code, logged in: https://code.claude.com/docs
      Check your setup with: grove doctor
      After brew upgrade grove, press R on the dashboard to restart it.
    EOS
  end

  test do
    assert_match "grove #{version}", shell_output("#{bin}/grove --version")
    ENV["GROVE_HOME"] = testpath/"grove"
    system bin/"grove", "add", testpath
    assert_path_exists testpath/"grove/projects"
  end
end
EOF
