#!/usr/bin/env bash
# Serves the AudioBoard website locally and opens it in the browser.
#
#   website/serve.sh            # http://localhost:8000 (or the next free port)
#   website/serve.sh 9000       # a port of your choice
#   website/serve.sh --no-open  # don't open the browser
#
# Ctrl+C stops the server.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
self="$here/$(basename "$0")"
cd "$here"

port="${PORT:-8000}"
open_browser=1
for arg in "$@"; do
  case "$arg" in
    --no-open) open_browser=0 ;;
    -h|--help) sed -n '2,8p' "$self" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *[!0-9]*) echo "Unknown option: $arg (try --help)" >&2; exit 1 ;;
    *) port="$arg" ;;
  esac
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is needed to serve the site (or open website/index.html directly)." >&2
  exit 1
fi

# Take the next port if this one is busy.
busy() { python3 -c "import socket,sys; s=socket.socket(); sys.exit(0 if s.connect_ex(('127.0.0.1', $1)) == 0 else 1)"; }
tries=0
while busy "$port"; do
  port=$((port + 1)); tries=$((tries + 1))
  if [ "$tries" -ge 20 ]; then echo "No free port found." >&2; exit 1; fi
done

url="http://localhost:$port/"
echo "AudioBoard website: $url  (Ctrl+C to stop)"

if [ "$open_browser" = 1 ]; then
  opener=""
  if command -v open >/dev/null 2>&1; then opener=open
  elif command -v xdg-open >/dev/null 2>&1; then opener=xdg-open; fi
  if [ -n "$opener" ]; then (sleep 1 && "$opener" "$url" >/dev/null 2>&1) & fi
fi

exec python3 -m http.server "$port" --bind 127.0.0.1
