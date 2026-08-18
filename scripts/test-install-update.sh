#!/usr/bin/env bash
# Integration test for install.sh / update.sh using a local mock GitHub release.
# shellcheck shell=bash

set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

mock_dir=$(mktemp -d)
SERVER_PID=""
trap '[[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" >/dev/null 2>&1 || true; rm -rf "$mock_dir"' EXIT

mkdir -p "$mock_dir/api/releases/tags" "$mock_dir/v1.3.15" "$mock_dir/v1.3.16"

# Happy-path release 1.3.15
printf 'new binary' > "$mock_dir/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu"
chmod +x "$mock_dir/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu"
size=$(stat -c %s "$mock_dir/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu")
sha=$(sha256sum "$mock_dir/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu" | awk '{print $1}')

cat > "$mock_dir/v1.3.15/squaremap-backends.json" <<EOF
{
  "pluginVersion": "1.3.15",
  "targets": {
    "x86_64-unknown-linux-gnu": {
      "url": "http://localhost:18080/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu",
      "length": "$size",
      "sha256": "$sha"
    }
  }
}
EOF
manifest_size=$(stat -c %s "$mock_dir/v1.3.15/squaremap-backends.json")

cat > "$mock_dir/api/releases/tags/v1.3.15" <<EOF
{
  "tag_name": "v1.3.15",
  "assets": [
    { "name": "squaremap-server-x86_64-unknown-linux-gnu", "size": $size, "browser_download_url": "http://localhost:18080/v1.3.15/squaremap-server-x86_64-unknown-linux-gnu" },
    { "name": "squaremap-backends.json", "size": $manifest_size, "browser_download_url": "http://localhost:18080/v1.3.15/squaremap-backends.json" }
  ]
}
EOF

# Rollback test release 1.3.16
printf 'new binary x' > "$mock_dir/v1.3.16/squaremap-server-x86_64-unknown-linux-gnu"
chmod +x "$mock_dir/v1.3.16/squaremap-server-x86_64-unknown-linux-gnu"
printf 'new binary arm' > "$mock_dir/v1.3.16/squaremap-server-aarch64-unknown-linux-gnu"
chmod +x "$mock_dir/v1.3.16/squaremap-server-aarch64-unknown-linux-gnu"

cat > "$mock_dir/api/releases/tags/v1.3.16" <<EOF
{
  "tag_name": "v1.3.16",
  "assets": [
    { "name": "squaremap-server-x86_64-unknown-linux-gnu", "size": 12, "browser_download_url": "http://localhost:18080/v1.3.16/squaremap-server-x86_64-unknown-linux-gnu" },
    { "name": "squaremap-server-aarch64-unknown-linux-gnu", "size": 999, "browser_download_url": "http://localhost:18080/v1.3.16/squaremap-server-aarch64-unknown-linux-gnu" }
  ]
}
EOF

# start mock server
cd "$mock_dir"
python3 -m http.server 18080 > /tmp/test-install-update-server.log 2>&1 &
SERVER_PID=$!
sleep 1
cd "$repo_root"

fail() { echo "[FAIL] $*" >&2; exit 1; }
pass() { echo "[PASS] $*"; }

# Test fresh install with API + manifest
rm -rf rust/backend
SQUAREMAP_API_URL=http://localhost:18080/api ./install.sh 1.3.15 --target x86_64-unknown-linux-gnu
content=$(cat rust/backend/rust-backend-x86_64-unknown-linux-gnu/squaremap-server-x86_64-unknown-linux-gnu)
[[ "$content" == "new binary" ]] || fail "fresh install content mismatch: $content"
pass "fresh install"

# Set a sentinel value so we can detect whether rollback restores old binary
printf 'old sentinel' > rust/backend/rust-backend-x86_64-unknown-linux-gnu/squaremap-server-x86_64-unknown-linux-gnu
SQUAREMAP_API_URL=http://localhost:18080/api ./update.sh 1.3.16 --target x86_64-unknown-linux-gnu --target aarch64-unknown-linux-gnu && fail "update should have failed on second target size"
content=$(cat rust/backend/rust-backend-x86_64-unknown-linux-gnu/squaremap-server-x86_64-unknown-linux-gnu)
[[ "$content" == "old sentinel" ]] || fail "rollback did not restore old binary: $content"
pass "rollback"

# Ensure no leftover staging or backup dirs
if ls -d rust/backend/.update-* >/dev/null 2>&1; then
  fail "leftover .update-* staging directory"
fi
if ls -d rust/backend/*.old-* >/dev/null 2>&1; then
  fail "leftover .old-* backup directory"
fi

pass "no leftover staging/backup directories"
