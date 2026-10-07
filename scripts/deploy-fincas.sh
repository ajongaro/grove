#!/usr/bin/env bash
# Build and install grove on the fincas, then print each one's version.
#   scripts/deploy-fincas.sh            # both
#   scripts/deploy-fincas.sh therver    # just one (therver | pi)
# Overridable: MAC_FINCA (default therver@therver.local), PI_FINCA (admin@pi-five-ssd.local).
# Each finca's dashboard restarts on its own the next time grove is opened there.
set -euo pipefail
cd "$(dirname "$0")/.."

MAC_FINCA=${MAC_FINCA:-therver@therver.local}
PI_FINCA=${PI_FINCA:-admin@pi-five-ssd.local}
targets=${1:-all}

# This laptop's Xcode toolchain is broken; the CLT + 26.5 SDK link fine.
if [[ -d /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk ]]; then
  export DEVELOPER_DIR=/Library/Developer/CommandLineTools
  export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
fi

if [[ $targets == all || $targets == therver ]]; then
  echo "== therver: building macOS arm64 locally"
  cargo build --release --quiet
  ssh "$MAC_FINCA" 'mkdir -p ~/.local/bin'
  scp -q target/release/grove "$MAC_FINCA:.local/bin/grove.new"
  # Rename over the old binary so a running grove keeps its (old) inode.
  ssh "$MAC_FINCA" 'mv -f ~/.local/bin/grove.new ~/.local/bin/grove && ~/.local/bin/grove --version'
fi

if [[ $targets == all || $targets == pi ]]; then
  echo "== pi-five-ssd: syncing source and building on the Pi"
  ssh "$PI_FINCA" 'mkdir -p ~/src/grove'
  rsync -a --delete --exclude target --exclude .git --exclude .superpowers ./ "$PI_FINCA:src/grove/"
  ssh "$PI_FINCA" 'set -e; . ~/.cargo/env; cd ~/src/grove; cargo build --release --quiet; mkdir -p ~/.local/bin; install -m 755 target/release/grove ~/.local/bin/grove.new; mv -f ~/.local/bin/grove.new ~/.local/bin/grove; ~/.local/bin/grove --version'
fi
