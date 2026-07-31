#!/usr/bin/env bash
# Build the Rust hooks and prove they behave identically to the JS ones.
#
# The differential test is not optional here: a regex-dialect divergence between
# JavaScript and Rust is silent, and this is the only thing that catches it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/rust/target/release/omp-hooks"

command -v cargo >/dev/null || {
  echo "ERROR: cargo not found. Install Rust: https://rustup.rs" >&2
  exit 1
}
command -v node >/dev/null || {
  echo "ERROR: node is required to run the differential test against the JS hooks." >&2
  exit 1
}

echo "==> building (release)"
( cd "$ROOT/rust" && cargo build --release )

echo
echo "==> rust unit tests"
( cd "$ROOT/rust" && cargo test --quiet )

echo
echo "==> differential test: rust output must match node byte-for-byte"
node "$ROOT/test/differential.js" "$BIN"

echo "==> size: $(ls -lh "$BIN" | awk '{print $5}')"
echo
echo "Built: $BIN"
echo "Activate with: bash install.sh    (it prefers the binary when present)"
