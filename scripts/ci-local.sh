#!/usr/bin/env bash
# Runs what .github/workflows/ci.yml runs, locally.
# Prerequisites: rustup toolchains `stable` and `1.75.0` with clippy + rustfmt, and the
# x86_64-unknown-linux-musl target on stable. `scripts/bootstrap-toolchain.sh` installs all of
# them privately under .toolchains/ without touching ~/.rustup; this script sources that env if present.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
[ -f .toolchains/env ] && . .toolchains/env
for tc in stable 1.75.0; do
  rustup run "$tc" cargo --version >/dev/null 2>&1 || {
    echo "missing rustup toolchain '$tc': run scripts/bootstrap-toolchain.sh (private) or 'rustup toolchain install $tc'" >&2; exit 2; }
done
rustup target list --installed --toolchain stable 2>/dev/null | grep -q x86_64-unknown-linux-musl || {
  echo "missing target x86_64-unknown-linux-musl on stable: run scripts/bootstrap-toolchain.sh or 'rustup target add x86_64-unknown-linux-musl'" >&2; exit 2; }
cargo +stable fmt --all -- --check
cargo +stable clippy --all-targets --locked -- -D warnings
cargo +1.75.0 test --locked
cargo +stable test --locked
grep -q '^version = 3$' Cargo.lock
cargo +stable build --release --locked --target x86_64-unknown-linux-musl
scripts/check-static.sh target/x86_64-unknown-linux-musl/release/xe-gmi
cargo +stable publish --dry-run --locked --allow-dirty
echo "ci-local: all green"
