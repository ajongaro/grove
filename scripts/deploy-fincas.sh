#!/usr/bin/env bash
# Install this checkout's grove here, then put it on your fincas with
# `grove finca update` and print each one's version.
#   scripts/deploy-fincas.sh              # every finca in ~/.grove/fincas
#   scripts/deploy-fincas.sh homebox pi   # just these (finca names)
# A finca with this machine's OS and CPU gets a copy of this build; any
# other builds from source there (needs cargo on it). Each one tries its
# `ssh` destination, then its `backup`.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo install --quiet --path .
exec "${CARGO_HOME:-$HOME/.cargo}/bin/grove" finca update "$@"
