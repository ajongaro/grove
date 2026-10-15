#!/usr/bin/env bash
# Releases grove:  scripts/release.sh 0.2.0
#  1. sets that version in Cargo.toml, runs the tests, commits and tags v0.2.0;
#  2. pushes, which starts .github/workflows/release.yml, and waits for it to
#     publish the binaries, SHA256SUMS and install.sh;
#  3. writes the new Formula/grove.rb (scripts/formula.sh) to the
#     ajongaro/homebrew-grove tap and pushes it.
# Needs a clean main and gh logged in. If step 3 fails, re-run with
# --formula-only to retry just that.
set -euo pipefail
cd "$(dirname "$0")/.."
die() { echo "release: $*" >&2; exit 1; }

formula_only=false
if [[ "${1:-}" == --formula-only ]]; then formula_only=true; shift; fi
version="${1:?usage: scripts/release.sh [--formula-only] <version, e.g. 0.2.0>}"
version="${version#v}"
tag="v$version"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "$version isn't a version like 0.2.0"

if ! $formula_only; then
  [[ "$(git branch --show-current)" == main ]] || die "not on main"
  [[ -z "$(git status --porcelain)" ]] || die "commit or stash your changes first"
  ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null || die "$tag already exists"

  # Only the first `version =` line: the [package] one.
  perl -0pi -e "s/^version = \"[^\"]*\"/version = \"$version\"/m" Cargo.toml
  cargo test --quiet
  cargo clippy --quiet --all-targets -- -D warnings
  if [[ -n "$(git status --porcelain)" ]]; then
    git commit --quiet -am "release: $tag"
  fi
  git tag -a "$tag" -m "grove $version"
  git push --quiet origin main "$tag"

  echo "Waiting for the release workflow…"
  run=""
  for _ in $(seq 30); do
    run="$(gh run list --workflow release.yml --json databaseId,headBranch \
      -q ".[] | select(.headBranch == \"$tag\") | .databaseId" | head -1)"
    [[ -n "$run" ]] && break
    sleep 2
  done
  [[ -n "$run" ]] || die "no release workflow run started for $tag"
  gh run watch "$run" --exit-status >/dev/null || die "the release workflow failed: gh run view $run --log-failed"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gh release download "$tag" -p SHA256SUMS -D "$tmp"
gh repo clone ajongaro/homebrew-grove "$tmp/tap" -- --quiet
mkdir -p "$tmp/tap/Formula"
scripts/formula.sh "$version" "$tmp/SHA256SUMS" > "$tmp/tap/Formula/grove.rb"
git -C "$tmp/tap" add Formula/grove.rb
if git -C "$tmp/tap" diff --cached --quiet; then
  echo "The tap already has grove $version."
else
  git -C "$tmp/tap" commit --quiet -m "grove $version"
  git -C "$tmp/tap" push --quiet
fi
echo "Released $tag: brew upgrade grove, or re-run install.sh."
