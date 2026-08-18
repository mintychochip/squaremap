#!/usr/bin/env bash
# Common functions for install.sh and update.sh.
# shellcheck shell=bash

set -euo pipefail

readonly SQUAREMAP_REPO="jpenilla/squaremap"
readonly SQUAREMAP_API_URL="${SQUAREMAP_API_URL:-https://api.github.com/repos/${SQUAREMAP_REPO}}"
readonly SQUAREMAP_DOWNLOAD_URL_BASE="${SQUAREMAP_DOWNLOAD_URL_BASE:-https://github.com/${SQUAREMAP_REPO}/releases/download}"

readonly SUPPORTED_TARGETS=(
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  x86_64-pc-windows-msvc
  x86_64-apple-darwin
  aarch64-apple-darwin
)

info() { echo "[*] $*" >&2; }
warn() { echo "[!] $*" >&2; }
die() { echo "[!!] $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: install.sh|update.sh [OPTIONS] [VERSION]

Options:
  --all-targets            Download all supported targets.
  --target <TRIPLE>        Download a specific target (repeatable).
  --force                  Overwrite existing directories (install only).
  --insecure               Skip GitHub API and manifest verification.
  --manifest <PATH>        Use a local squaremap-backends.json for verification.
  --github-token <TOKEN>   GitHub token for API authentication.
  -h, --help               Show this help.

VERSION defaults to the value in gradle.properties (with -SNAPSHOT stripped).
EOF
}

resolve_version() {
  local version="${1:-${VERSION:-}}"
  if [[ -z "$version" && -f gradle.properties ]]; then
    version=$(sed -n 's/^version=//p' gradle.properties | head -n1)
  fi
  if [[ -z "$version" ]]; then
    die "No version specified. Pass a version, set VERSION, or run from the repo root."
  fi
  printf '%s\n' "${version%-SNAPSHOT}"
}

host_target() {
  if command -v rustc >/dev/null 2>&1; then
    rustc -vV | sed -n 's/^host: //p'
    return
  fi
  local arch kernel
  arch=$(uname -m)
  kernel=$(uname -s)
  case "$kernel" in
    Linux) kernel=unknown-linux ;;
    Darwin) kernel=apple ;;
    CYGWIN*|MINGW*|MSYS*) kernel=pc-windows ;;
    *) die "Unsupported kernel: $kernel" ;;
  esac
  case "$arch" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
  esac
  local libc=
  if [[ "$kernel" == "pc-windows" ]]; then
    libc=-msvc
  elif [[ "$kernel" == "unknown-linux" ]]; then
    libc=-gnu
  fi
  if [[ "$kernel" == "apple" ]]; then
    echo "${arch}-${kernel}-darwin"
  else
    echo "${arch}-${kernel}${libc}"
  fi
}

is_target_supported() {
  local t
  for t in "${SUPPORTED_TARGETS[@]}"; do
    [[ "$t" == "$1" ]] && return 0
  done
  return 1
}

binary_name_for_target() {
  local target="$1" exe=""
  [[ "$target" == *"windows"* ]] && exe=".exe"
  printf 'squaremap-server-%s%s\n' "$target" "$exe"
}

download_asset() {
  local url="$1" dest="$2"
  mkdir -p "$(dirname "$dest")"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$dest" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$dest" "$url"
  else
    die "curl or wget is required"
  fi
}

file_size() {
  local f="$1"
  if command -v stat >/dev/null 2>&1; then
    if stat -c %s "$f" >/dev/null 2>&1; then
      stat -c %s "$f"
    else
      stat -f %z "$f"
    fi
  else
    wc -c < "$f" | tr -d ' \n'
  fi
}

sha256_file() {
  local f="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$f" | awk '{print $1}'
  else
    shasum -a 256 "$f" | awk '{print $1}'
  fi
}

fetch_release_json() {
  local version="$1" token="${2:-}"
  local url="${SQUAREMAP_API_URL}/releases/tags/v${version}"
  local tmp
  tmp=$(mktemp)
  local headers=(-H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")
  [[ -n "$token" ]] && headers+=(-H "Authorization: Bearer ${token}")
  if ! curl -fsSL --retry 3 "${headers[@]}" -o "$tmp" "$url" 2>/dev/null; then
    if ! wget -q --header="Accept: application/vnd.github+json" --header="X-GitHub-Api-Version: 2022-11-28" ${token:+--header="Authorization: Bearer ${token}"} -O "$tmp" "$url" 2>/dev/null; then
      rm -f "$tmp"
      die "Failed to fetch release metadata from ${url}"
    fi
  fi
  printf '%s\n' "$tmp"
}

get_asset_size_url() {
  local release_json="$1" asset_name="$2"
  local output
  if command -v jq >/dev/null 2>&1; then
    output=$(jq -r --arg name "$asset_name" '.assets[] | select(.name == $name) | "\(.size) \(.browser_download_url)"' "$release_json")
  elif command -v python3 >/dev/null 2>&1; then
    output=$(python3 - "$release_json" "$asset_name" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
for a in data.get('assets', []):
    if a.get('name') == sys.argv[2]:
        print(a.get('size', ''), a.get('browser_download_url', ''))
        break
PY
)
  elif command -v python >/dev/null 2>&1; then
    output=$(python - "$release_json" "$asset_name" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
for a in data.get('assets', []):
    if a.get('name') == sys.argv[2]:
        print(a.get('size', ''), a.get('browser_download_url', ''))
        break
PY
)
  else
    die "jq or python3 is required to parse GitHub release metadata (or pass --insecure)"
  fi
  if [[ -z "$output" ]]; then
    return 1
  fi
  printf '%s\n' "$output"
}

get_asset_url() {
  local output
  output=$(get_asset_size_url "$1" "$2") || return 1
  awk '{print $2}' <<< "$output"
}

verify_size() {
  local file="$1" expected="$2"
  local actual
  actual=$(file_size "$file")
  [[ "$actual" -eq "$expected" ]] || die "Size mismatch for $(basename "$file"): expected ${expected}, got ${actual}"
}

verify_with_manifest() {
  local manifest="$1" target="$2" binary="$3" version="$4"
  local plugin_version expected_length expected_sha256 actual_length actual_sha256
  if command -v jq >/dev/null 2>&1; then
    plugin_version=$(jq -r '.pluginVersion' "$manifest")
    expected_length=$(jq -r --arg t "$target" '.targets[$t].length' "$manifest")
    expected_sha256=$(jq -r --arg t "$target" '.targets[$t].sha256' "$manifest")
  elif command -v python3 >/dev/null 2>&1; then
    read -r plugin_version expected_length expected_sha256 < <(python3 - "$manifest" "$target" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
t = sys.argv[2]
print(data['pluginVersion'], data['targets'][t]['length'], data['targets'][t]['sha256'])
PY
)
  elif command -v python >/dev/null 2>&1; then
    read -r plugin_version expected_length expected_sha256 < <(python - "$manifest" "$target" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
t = sys.argv[2]
print(data['pluginVersion'], data['targets'][t]['length'], data['targets'][t]['sha256'])
PY
)
  else
    die "jq or python3 is required to parse the manifest"
  fi
  [[ -n "$plugin_version" && "$plugin_version" != "null" ]] || die "Manifest is missing pluginVersion"
  [[ -n "$expected_length" && "$expected_length" != "null" ]] || die "Manifest entry missing for $target"
  [[ -n "$expected_sha256" && "$expected_sha256" != "null" ]] || die "Manifest SHA-256 missing for $target"
  [[ "$plugin_version" == "$version" ]] || die "Manifest pluginVersion mismatch: expected $version, got $plugin_version"
  actual_length=$(file_size "$binary")
  [[ "$actual_length" -eq "$expected_length" ]] || die "Manifest length mismatch for $target"
  actual_sha256=$(sha256_file "$binary")
  [[ "$actual_sha256" == "$expected_sha256" ]] || die "Manifest SHA-256 mismatch for $target"
}

# shellcheck disable=SC2317
cleanup() {
  local status=$?
  if [[ -n "${TMP_STAGE:-}" && -f "$TMP_STAGE/moved" ]]; then
    while IFS='|' read -r final_dir old_final; do
      if [[ $status -ne 0 ]]; then
        if [[ -e "$old_final" ]]; then
          rm -rf "$final_dir"
          mv "$old_final" "$final_dir"
          rm -rf "$(dirname "$old_final")"
        fi
      else
        rm -rf "$(dirname "$old_final")"
      fi
    done < "$TMP_STAGE/moved"
  fi
  rm -rf "${TMP_STAGE:-}" "${RELEASE_JSON:-}"
}

do_install_or_update() {
  local mode="$1"
  shift

  local version_arg="" force=0 insecure=0 manifest_path="" token=""
  local all_targets=0
  local -a targets=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --all-targets) all_targets=1 ;;
      --target) shift; [[ $# -gt 0 ]] || die "--target requires a value"; targets+=("$1") ;;
      --force) force=1 ;;
      --insecure) insecure=1 ;;
      --manifest) shift; [[ $# -gt 0 ]] || die "--manifest requires a value"; manifest_path="$1" ;;
      --github-token) shift; [[ $# -gt 0 ]] || die "--github-token requires a value"; token="$1" ;;
      -h|--help) usage; exit 0 ;;
      --*) die "Unknown option: $1" ;;
      *)
        [[ -z "$version_arg" ]] || die "Unexpected argument: $1"
        version_arg="$1"
        ;;
    esac
    shift
  done

  local version
  version=$(resolve_version "$version_arg")

  if [[ $all_targets -eq 1 ]]; then
    targets=("${SUPPORTED_TARGETS[@]}")
  elif [[ ${#targets[@]} -eq 0 ]]; then
    targets=("$(host_target)")
  fi

  local t
  for t in "${targets[@]}"; do
    is_target_supported "$t" || die "Unsupported target: $t"
  done

  info "Resolving release v${version}"
  mkdir -p rust/backend

  local stage_dir
  stage_dir=$(mktemp -d "rust/backend/.update-XXXXXX")
  export TMP_STAGE="$stage_dir"
  export RELEASE_JSON=""
  trap 'cleanup' EXIT

  local release_json=""
  local manifest_tmp=""
  if [[ $insecure -eq 0 ]]; then
    release_json=$(fetch_release_json "$version" "$token")
    export RELEASE_JSON="$release_json"
    if [[ -n "$manifest_path" ]]; then
      manifest_tmp="$manifest_path"
    elif manifest_url=$(get_asset_url "$release_json" "squaremap-backends.json" 2>/dev/null) && [[ -n "$manifest_url" ]]; then
      manifest_tmp="$TMP_STAGE/squaremap-backends.json"
      download_asset "$manifest_url" "$manifest_tmp"
      info "Downloaded release manifest for verification"
    fi
  fi

  local target binary_name size url stage_dest
  for target in "${targets[@]}"; do
    binary_name=$(binary_name_for_target "$target")
    stage_dest="$TMP_STAGE/rust-backend-$target/$binary_name"

    if [[ $insecure -eq 0 ]]; then
      if ! read -r size url < <(get_asset_size_url "$release_json" "$binary_name"); then
        die "No release asset found for $binary_name"
      fi
      [[ -n "$url" ]] || die "No download URL for $binary_name"
    else
      size=""
      url="${SQUAREMAP_DOWNLOAD_URL_BASE}/v${version}/${binary_name}"
    fi

    info "Downloading $binary_name"
    mkdir -p "$(dirname "$stage_dest")"
    download_asset "$url" "$stage_dest"

    if [[ $insecure -eq 0 ]]; then
      verify_size "$stage_dest" "$size"
      if [[ -n "$manifest_tmp" ]]; then
        verify_with_manifest "$manifest_tmp" "$target" "$stage_dest" "$version"
      fi
    fi

    if [[ "$binary_name" != *.exe ]]; then
      chmod +x "$stage_dest"
    fi
  done

  for target in "${targets[@]}"; do
    binary_name=$(binary_name_for_target "$target")
    local final_dir="rust/backend/rust-backend-$target"
    if [[ "$mode" == "install" && -e "$final_dir" && $force -eq 0 ]]; then
      die "Directory $final_dir already exists. Use --force to overwrite or run update.sh."
    fi

    local old_dir old_final
    if [[ -e "$final_dir" ]]; then
      old_dir=$(mktemp -d "rust/backend/rust-backend-$target.old-XXXXXX")
      old_final="$old_dir/final"
      mv "$final_dir" "$old_final"
      echo "$final_dir|$old_final" >> "$TMP_STAGE/moved"
    fi
    mv "$TMP_STAGE/rust-backend-$target" "$final_dir"
    info "Installed $binary_name into $final_dir"
  done
}
