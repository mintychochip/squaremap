#!/usr/bin/env bash
# Paired Java/Rust A/B orchestrator: three repeats per workload, checksum-gated graphs.
# shellcheck shell=bash

set -euo pipefail

info() { echo "[*] $*" >&2; }
warn() { echo "[!] $*" >&2; }
die() { echo "[!!] $*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: scripts/ab-java-rust.sh [--workload chunk-render-v2|pyramid-png-v2|all]

Run the matched Java and Rust protocol benches three times each, write per-pass
JSON, generate squaremap-compare ab-report graphs, and copy the concatenated
summary into docs/superpowers/verification/ab/<workload>/.

Workloads:
  chunk-render-v2  Direct 26-case renderer loop (no PNG/IO in the timed region)
  pyramid-png-v2   Tile pyramid save/encode for the 10 catalog cases (PNG included)
  all              Both (default)

Comparison is checksum-gated. Do not treat Criterion HTML as this A/B.
EOF
}

repo_root=$(cd "$(dirname "$0")/.." && pwd)
if command -v git >/dev/null 2>&1 && git -C "$repo_root" rev-parse --show-toplevel >/dev/null 2>&1; then
  repo_root=$(git -C "$repo_root" rev-parse --show-toplevel)
fi
cd "$repo_root"

workload="all"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --workload)
      shift
      [[ $# -gt 0 ]] || die "--workload requires a value"
      workload="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --*)
      die "Unknown option: $1"
      ;;
    *)
      die "Unexpected argument: $1"
      ;;
  esac
  shift
done

case "$workload" in
  all|chunk-render-v2|pyramid-png-v2) ;;
  *) die "Unknown workload: $workload (expected chunk-render-v2|pyramid-png-v2|all)" ;;
esac

command -v python3 >/dev/null 2>&1 || die "python3 is required to concatenate pass samples"
[[ -x ./gradlew ]] || die "gradlew not found in $repo_root"

mismatch=0

write_summary() {
  local dest="$1"
  shift
  python3 - "$dest" "$@" <<'PY'
import json
import sys
from pathlib import Path

dest = Path(sys.argv[1])
runs = []
for path in sys.argv[2:]:
    with open(path, encoding="utf-8") as handle:
        runs.append(json.load(handle))
if not runs:
    raise SystemExit("no sample files")
base = dict(runs[0])
pass_nanos = []
elapsed = 0
for run in runs:
    samples = run.get("pass_nanos")
    if not isinstance(samples, list) or not samples:
        raise SystemExit(f"{run.get('backend')} sample is missing pass_nanos")
    pass_nanos.extend(samples)
    elapsed += int(run["elapsed_nanos"])
measured = len(pass_nanos)
case_count = int(base["case_count"])
base["pass_nanos"] = pass_nanos
base["measured_passes"] = measured
base["elapsed_nanos"] = elapsed
base["items_per_second"] = case_count * measured * 1_000_000_000.0 / elapsed
dest.parent.mkdir(parents=True, exist_ok=True)
dest.write_text(json.dumps(base, separators=(",", ":")) + "\n", encoding="utf-8")
PY
}

run_ab_report() {
  local java_json="$1" rust_json="$2" out_dir="$3"
  local status=0
  cargo run --manifest-path rust/Cargo.toml -p squaremap-compare --release -- \
    ab-report --java "$java_json" --rust "$rust_json" --output "$out_dir" || status=$?
  if [[ $status -eq 0 ]]; then
    return 0
  fi
  if [[ $status -eq 1 ]]; then
    warn "checksum mismatch in $out_dir (report still written)"
    mismatch=1
    return 0
  fi
  die "ab-report failed with status $status for $out_dir"
}

run_java() {
  local name="$1" out="$2"
  case "$name" in
    chunk-render-v2)
      ./gradlew :squaremap-common:test --tests '*ChunkRenderBenchmarkTest' \
        -Dsquaremap.renderBenchmark=true -Dsquaremap.abOut="$out" \
        --no-daemon --console=plain --rerun-tasks
      ;;
    pyramid-png-v2)
      ./gradlew :squaremap-common:test --tests '*ImagePyramidBenchmarkTest' \
        -Dsquaremap.tileBenchmark=true -Dsquaremap.abOut="$out" \
        --no-daemon --console=plain --rerun-tasks
      ;;
    *)
      die "Unknown Java workload: $name"
      ;;
  esac
}

run_rust() {
  local name="$1" out="$2"
  case "$name" in
    chunk-render-v2)
      SQUAREMAP_AB_OUT="$out" cargo run --manifest-path rust/Cargo.toml \
        -p squaremap-render --bin render_protocol --release
      ;;
    pyramid-png-v2)
      SQUAREMAP_AB_OUT="$out" cargo run --manifest-path rust/Cargo.toml \
        -p squaremap-render --bin tile_protocol --release
      ;;
    *)
      die "Unknown Rust workload: $name"
      ;;
  esac
}

run_workload() {
  local name="$1"
  local work_dir="$repo_root/build/ab/$name"
  mkdir -p "$work_dir"
  info "Workload $name (3 repeats) -> $work_dir"

  local i java_json rust_json
  for i in 1 2 3; do
    java_json="$work_dir/java-run-$i.json"
    rust_json="$work_dir/rust-run-$i.json"
    info "$name Java run $i"
    run_java "$name" "$java_json"
    [[ -f "$java_json" ]] || die "Java did not write $java_json"
    info "$name Rust run $i"
    run_rust "$name" "$rust_json"
    [[ -f "$rust_json" ]] || die "Rust did not write $rust_json"
    info "$name ab-report run $i"
    run_ab_report "$java_json" "$rust_json" "$work_dir/run-$i"
  done

  write_summary "$work_dir/java-summary.json" \
    "$work_dir/java-run-1.json" "$work_dir/java-run-2.json" "$work_dir/java-run-3.json"
  write_summary "$work_dir/rust-summary.json" \
    "$work_dir/rust-run-1.json" "$work_dir/rust-run-2.json" "$work_dir/rust-run-3.json"
  info "$name ab-report summary (90 samples)"
  run_ab_report "$work_dir/java-summary.json" "$work_dir/rust-summary.json" "$work_dir/summary"

  local docs_dir="$repo_root/docs/superpowers/verification/ab/$name"
  mkdir -p "$docs_dir"
  cp "$work_dir/summary/index.html" "$docs_dir/index.html"
  cp "$work_dir/summary/throughput.svg" "$docs_dir/throughput.svg"
  cp "$work_dir/summary/pass-times.svg" "$docs_dir/pass-times.svg"
  cp "$work_dir/summary/ab-report.json" "$docs_dir/ab-report.json"
  info "Copied summary artifacts to $docs_dir"
}

info "Prebuilding release binaries"
cargo build --manifest-path rust/Cargo.toml --release -p squaremap-render -p squaremap-compare

case "$workload" in
  all)
    run_workload chunk-render-v2
    run_workload pyramid-png-v2
    ;;
  *)
    run_workload "$workload"
    ;;
esac

if [[ $mismatch -ne 0 ]]; then
  warn "One or more pairs had passed=false; do not publish a speedup"
  exit 1
fi
info "A/B reports written under docs/superpowers/verification/ab/"
