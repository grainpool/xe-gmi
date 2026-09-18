#!/usr/bin/env bash
# Fails unless the given binary is a statically linked, stripped Linux executable.
set -euo pipefail
BIN="${1:?binary path}"
file "$BIN" | tee /dev/stderr | grep -qE 'statically linked|static-pie linked' || { echo "not static" >&2; exit 1; }
if command -v ldd >/dev/null; then ldd "$BIN" 2>&1 | grep -qiE 'not a dynamic executable|statically linked' || { echo "ldd shows dynamic deps" >&2; exit 1; }; fi
file "$BIN" | grep -q 'not stripped' && { echo "not stripped" >&2; exit 1; }
"$BIN" --version
echo "static ok: $BIN"
