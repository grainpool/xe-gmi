#!/usr/bin/env bash
# Scan the working tree for personal data and vendor tool names before a commit. Exit 1 on findings.
# The synthetic /home/user/... strings used by tests are whitelisted; captured fixtures are gitignored
# but are scanned too when present.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 2
X=(--exclude-dir=.git --exclude-dir=target --exclude-dir=synthetic --exclude-dir=.toolchains --exclude-dir=dev --exclude-dir=__pycache__ --exclude=pii-scan.sh)
findings=""
add() { [ -n "$1" ] && findings+="$1"$'\n'; }
# real home directories (synthetic ones are whitelisted)
add "$(grep -rnE "${X[@]}" '/home/[a-z][a-z0-9_-]*' . | grep -vE '/home/(user|runner)/' | sed 's/^/home dir: /')"
# e-mail addresses other than the commit identity
add "$(grep -rnoE "${X[@]}" '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[a-z]{2,}' . | grep -vE 'grainpool@users\.noreply\.github\.com|noreply@|example\.(com|org)|@[a-z.]*kernel\.org' | sed 's/^/e-mail: /')"
# IPv4 addresses (kernel version strings and dotted numbers below 256 in every octet only)
add "$(grep -rnoE "${X[@]}" '\b([0-9]{1,3}\.){3}[0-9]{1,3}\b' . | grep -vE '(127\.0\.0\.1|0\.0\.0\.0|[0-9]+\.[0-9]+\.[0-9]+\.(fc|el)|\b[67]\.[0-9]+\.[0-9]+\.[0-9]+\b|\b1\.[0-9]+\.[0-9]+\.[0-9]+\b)' | sed 's/^/ip: /')"
# hostnames recorded by the capture script
add "$(grep -rnE --exclude-dir=.git 'hostname=' fixtures/captured 2>/dev/null | sed 's/^/hostname: /')"
# vendor tool names are never mentioned
add "$(grep -rniE "${X[@]}" '(nvidia-smi|xpu-smi|xpumanager|rocm-smi|intel_gpu_top)' . | sed 's/^/tool name: /')"
# bystander GPU ids: non-Intel fixture devices carry only the synthetic placeholder id (ffff);
# a real other-vendor device id in the tree means rig hardware identity leaked into a fixture
X2=(--exclude-dir=.git --exclude-dir=target --exclude-dir=captured --exclude=pii-scan.sh --exclude='results-*.md' --exclude='events-*.log')
add "$(grep -rnoE "${X2[@]}" '(10de|1002):[0-9a-f]{4}|0x(10de|1002), 0x[0-9a-f]{4}' fixtures tests src docs scripts verify | grep -v ffff | sed 's/^/bystander pci id: /')"
add "$(grep -rnoE "${X2[@]}" '0x1462|\b(2bb1|2d04|204b|5340)\b' fixtures tests src docs scripts verify | sed 's/^/rig pci id: /')"
if [ -n "$findings" ]; then printf 'pii-scan: findings\n%s' "$findings"; exit 1; fi
echo "pii-scan: clean"
