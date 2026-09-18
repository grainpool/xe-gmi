#!/usr/bin/env bash
# Installs a private rustup into ./.toolchains (never touches ~/.cargo or ~/.rustup) with:
#   - the MSRV toolchain 1.75.0 (what Ubuntu 24.04 ships)
#   - stable
#   - musl targets for static builds (x86_64 native; aarch64 via `cross` in CI only)
#   - cargo-about is NOT installed; no extra cargo plugins are needed.
# Source ./.toolchains/env afterwards (or let scripts/ci-local.sh do it).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export RUSTUP_HOME="$ROOT/.toolchains/rustup" CARGO_HOME="$ROOT/.toolchains/cargo"
mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain stable
fi
export PATH="$CARGO_HOME/bin:$PATH"
rustup toolchain install 1.75.0 --profile minimal --component clippy,rustfmt
rustup toolchain install stable --profile minimal --component clippy,rustfmt
rustup target add x86_64-unknown-linux-musl --toolchain stable
rustup target add x86_64-unknown-linux-musl --toolchain 1.75.0
cat > "$ROOT/.toolchains/env" <<EOS
export RUSTUP_HOME="$RUSTUP_HOME"
export CARGO_HOME="$CARGO_HOME"
export PATH="$CARGO_HOME/bin:\$PATH"
EOS
echo "toolchains ready; run: source $ROOT/.toolchains/env"
rustup show
