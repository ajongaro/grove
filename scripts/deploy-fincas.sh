#!/usr/bin/env bash
# Build and install grove on your fincas, then print each one's version.
#   scripts/deploy-fincas.sh              # every finca in ~/.grove/fincas
#   scripts/deploy-fincas.sh homebox pi   # just these (finca file names)
# A finca with this machine's OS and CPU gets the local release build; any
# other builds from source there (needs cargo on it). Each one tries its
# `ssh` destination, then its `backup`. grove goes to the finca file's
# grove_bin. Each finca's dashboard restarts on its own the next time grove
# is opened there.
set -euo pipefail
cd "$(dirname "$0")/.."

fincas_dir=${GROVE_HOME:-$HOME/.grove}/fincas
ssh_opts=(-o BatchMode=yes -o ConnectTimeout=5)

# Prints a string value from a flat TOML file: field <file> <key>
field() { sed -n "s/^$2 *= *\"\(.*\)\"$/\1/p" "$1" | head -1; }

if [[ $# -gt 0 ]]; then
  files=()
  for name in "$@"; do files+=("$fincas_dir/$name.toml"); done
else
  shopt -s nullglob
  files=("$fincas_dir"/*.toml)
fi
[[ ${#files[@]} -gt 0 ]] || { echo "no fincas in $fincas_dir" >&2; exit 1; }

here="$(uname -s) $(uname -m | sed 's/aarch64/arm64/')"
built_here=no
failed=()

for f in "${files[@]}"; do
  [[ -f $f ]] || { echo "no finca file $f" >&2; failed+=("$f"); continue; }
  name=$(field "$f" name)
  bin=$(field "$f" grove_bin)
  dest=
  for d in "$(field "$f" ssh)" "$(field "$f" backup)"; do
    if [[ -n $d ]] && ssh "${ssh_opts[@]}" "$d" true 2>/dev/null; then
      dest=$d
      break
    fi
  done
  if [[ -z $dest ]]; then
    echo "== $name: unreachable, skipped" >&2
    failed+=("$name")
    continue
  fi

  there=$(ssh "${ssh_opts[@]}" "$dest" 'echo "$(uname -s) $(uname -m)"' | sed 's/aarch64/arm64/')
  # Rename over the old binary so a running grove keeps its (old) inode.
  swap="mv -f '$bin.new' '$bin' && '$bin' --version"
  if [[ $there == "$here" ]]; then
    echo "== $name ($dest, $there): copying the local build"
    if [[ $built_here == no ]]; then
      cargo build --release --quiet
      built_here=yes
    fi
    ssh "${ssh_opts[@]}" "$dest" "mkdir -p \"\$(dirname '$bin')\""
    scp -q target/release/grove "$dest:$bin.new"
    ssh "${ssh_opts[@]}" "$dest" "$swap"
  else
    echo "== $name ($dest, $there): building from source there"
    ssh "${ssh_opts[@]}" "$dest" 'mkdir -p ~/src/grove'
    rsync -a --delete --exclude target --exclude .git --exclude .superpowers ./ "$dest:src/grove/"
    ssh "${ssh_opts[@]}" "$dest" "set -e; if [ -f ~/.cargo/env ]; then . ~/.cargo/env; fi; cd ~/src/grove; cargo build --release --quiet; mkdir -p \"\$(dirname '$bin')\"; install -m 755 target/release/grove '$bin.new'; $swap"
  fi
done

if [[ ${#failed[@]} -gt 0 ]]; then
  echo "not deployed: ${failed[*]}" >&2
  exit 1
fi
