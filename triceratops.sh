#!/usr/bin/env bash
# Start Triceratops, rebuilding the release binary first when the sources changed.
#
#   ./triceratops.sh                     start Triceratops
#   ./triceratops.sh model.3dm           open a file
#   ./triceratops.sh --script s.txt      run command lines from a file at start-up
#   ./triceratops.sh --no-build ...      skip the cargo build check
#   TRICERATOPS_BACKEND=gl ./triceratops.sh   force a graphics backend
#
# File arguments are resolved against the directory you run the script from.
set -euo pipefail

ROOT="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
BIN="$ROOT/target/release/triceratops"

build=1
if [[ "${1:-}" == "--no-build" ]]; then
    build=0
    shift
fi

if (( build )); then
    if ! command -v cargo >/dev/null 2>&1; then
        if [[ -x "$BIN" ]]; then
            echo "triceratops: cargo not found, starting the existing build" >&2
        else
            echo "triceratops: cargo not found and no build at $BIN; install Rust from https://rustup.rs" >&2
            exit 1
        fi
    else
        # cargo only recompiles what changed; an up-to-date build takes about a second
        cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml" --bin triceratops
    fi
fi

if [[ ! -x "$BIN" ]]; then
    echo "triceratops: no binary at $BIN; run without --no-build" >&2
    exit 1
fi

exec "$BIN" "$@"
