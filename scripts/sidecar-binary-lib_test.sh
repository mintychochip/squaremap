#!/usr/bin/env bash
# Exercises download_asset from the shipped sidecar-binary-lib.sh.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=sidecar-binary-lib.sh
source "$script_dir/sidecar-binary-lib.sh"

scratch=$(mktemp -d "${TMPDIR:-/tmp}/squaremap-download-asset.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

# wget --quota does not limit a single file. This stand-in copies the
# whole HTTP body, matching GNU Wget 1.x and Wget2.
mkdir -p "$scratch/bin"
cat > "$scratch/bin/wget" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
out=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -O) out="$2"; shift 2 ;;
    -q|--quiet) shift ;;
    --quota|--quota=*)
      if [[ "$1" == --quota ]]; then shift 2; else shift; fi
      ;;
    http://*|https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
[[ -n "$out" && -n "$url" ]] || exit 2
python3 - "$url" "$out" <<'PY'
import sys, urllib.request
urllib.request.urlretrieve(sys.argv[1], sys.argv[2])
PY
EOF
chmod +x "$scratch/bin/wget"
ln -s "$(command -v python3)" "$scratch/bin/python3"
ln -s "$(command -v mkdir)" "$scratch/bin/mkdir"
ln -s "$(command -v dirname)" "$scratch/bin/dirname"
ln -s "$(command -v basename)" "$scratch/bin/basename"
ln -s "$(command -v stat)" "$scratch/bin/stat"
ln -s "$(command -v rm)" "$scratch/bin/rm"
ln -s "$(command -v wc)" "$scratch/bin/wc"
ln -s "$(command -v tr)" "$scratch/bin/tr"
ln -s "$(command -v cat)" "$scratch/bin/cat"
ln -s "$(command -v bash)" "$scratch/bin/bash"

python3 - "$scratch/httpd.pid" "$scratch/port" <<'PY' &
import sys, threading
from http.server import BaseHTTPRequestHandler, HTTPServer
pid_path, port_path = sys.argv[1], sys.argv[2]
body = b"A" * (1024 * 1024)
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *_args):
        pass
server = HTTPServer(("127.0.0.1", 0), Handler)
open(port_path, "w").write(str(server.server_address[1]))
open(pid_path, "w").write(str(__import__("os").getpid()))
server.serve_forever()
PY
for _ in $(seq 1 50); do
  [[ -f "$scratch/port" ]] && break
  sleep 0.05
done
port=$(cat "$scratch/port")
url="http://127.0.0.1:${port}/backend"
dest="$scratch/backend.bin"
max_bytes=7

set +e
PATH="$scratch/bin" download_asset "$url" "$dest" "$max_bytes"
status=$?
set -e

if [[ -f "$scratch/httpd.pid" ]]; then
  kill "$(cat "$scratch/httpd.pid")" 2>/dev/null || true
fi

if [[ $status -eq 0 && -f "$dest" ]]; then
  actual=$(wc -c < "$dest" | tr -d ' \n')
  if [[ "$actual" -gt "$max_bytes" ]]; then
    echo "download_asset wrote ${actual} bytes with max_bytes=${max_bytes} (wget single-file quota is not a bound)" >&2
    exit 1
  fi
fi
if [[ -f "$dest" ]]; then
  actual=$(wc -c < "$dest" | tr -d ' \n')
  if [[ "$actual" -gt "$max_bytes" ]]; then
    echo "oversized dest left behind: ${actual} bytes" >&2
    exit 1
  fi
fi
echo "download_asset capped wget-path download (status=${status})"

python3 - "$scratch/httpd2.pid" "$scratch/port2" <<'PY' &
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
pid_path, port_path = sys.argv[1], sys.argv[2]
body = b"backend"
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *_args):
        pass
server = HTTPServer(("127.0.0.1", 0), Handler)
open(port_path, "w").write(str(server.server_address[1]))
open(pid_path, "w").write(str(__import__("os").getpid()))
server.serve_forever()
PY
for _ in $(seq 1 50); do
  [[ -f "$scratch/port2" ]] && break
  sleep 0.05
done
port2=$(cat "$scratch/port2")
exact="$scratch/exact.bin"
PATH="$scratch/bin" download_asset "http://127.0.0.1:${port2}/backend" "$exact" 7
if [[ -f "$scratch/httpd2.pid" ]]; then
  kill "$(cat "$scratch/httpd2.pid")" 2>/dev/null || true
fi
exact_size=$(wc -c < "$exact" | tr -d ' \n')
[[ "$exact_size" -eq 7 ]] || { echo "exact download size ${exact_size}, expected 7" >&2; exit 1; }
[[ "$(cat "$exact")" == "backend" ]] || { echo "exact download body mismatch" >&2; exit 1; }
echo "download_asset accepted exact-length download"
