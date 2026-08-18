#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/sidecar-binary-lib.sh
source "$script_dir/scripts/sidecar-binary-lib.sh"

do_install_or_update install "$@"
