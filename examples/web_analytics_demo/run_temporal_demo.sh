#!/usr/bin/env bash
# One-command run of the time-series web-analytics demo: makes sure the
# three published days exist (building them via `build_temporal_demo.sh`
# on first use), writes `cubism serve`'s required placeholder cube-path
# parquet, starts the server, and prints (or opens) the dashboard URL --
# the "Time series" panel charts avg_revenue across 2026-04-06..08, with
# window 2026-04-07 at its corrected revision 2.
#
# Usage: ./run_temporal_demo.sh [--port N] [--rebuild] [--open]
#   --port N   server port (default 8080)
#   --rebuild  force a fresh data build even if one already exists
#              (note: `build_temporal_demo.sh` rm -rf's .temporal_build/)
#   --open     open the dashboard in the browser once ready
#
# Ctrl-C stops the server. Same ephemeral-output convention as the other
# scripts here: everything generated lands in gitignored `.temporal_build/`.
set -euo pipefail

cd "$(dirname "$0")"
OUT=.temporal_build
BIN="cargo run --release -q -p cubism-cli --"
SPEC=web_analytics_temporal.yaml
PORT=8080
REBUILD=0
OPEN_BROWSER=0

while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift 2 ;;
    --rebuild) REBUILD=1; shift ;;
    --open) OPEN_BROWSER=1; shift ;;
    *) echo "usage: $0 [--port N] [--rebuild] [--open]" >&2; exit 1 ;;
  esac
done

# ---- data -----------------------------------------------------------------
if [ "$REBUILD" = 1 ] || [ ! -f "$OUT/catalog.sqlite" ] || [ ! -f "$OUT/control.sqlite" ]; then
  echo "== building demo data (three days, revision-2 correction) =="
  ./build_temporal_demo.sh
else
  echo "== reusing existing demo data in $OUT (pass --rebuild to regenerate) =="
fi

# ---- placeholder cube-path parquet ----------------------------------------
# `serve` requires a real static-cube parquet positional even though
# `/api/series` never reads it (same trick query_temporal_demo.sh uses).
# `python3` may be a pyenv shim without pyarrow -- probe candidates.
PY=
for cand in python3 /opt/homebrew/Caskroom/miniforge/base/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do
  if "$cand" -c "import pyarrow" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "no python3 with pyarrow found (needed for the placeholder parquet)" >&2; exit 1; }
"$PY" - "$OUT/placeholder_cube.parquet" <<'PYEOF'
import sys
import pyarrow as pa
import pyarrow.parquet as pq

pq.write_table(pa.table({"xunit": ["/G"]}), sys.argv[1])
PYEOF

# ---- serve ------------------------------------------------------------------
echo "== starting cubism serve on :$PORT =="
$BIN serve "$OUT/placeholder_cube.parquet" --port "$PORT" \
  --spec "$SPEC" --warehouse "$OUT/warehouse" \
  --catalog-db "$OUT/catalog.sqlite" --control-db "$OUT/control.sqlite" \
  >"$OUT/serve_demo.log" 2>&1 &
SERVE_PID=$!
trap 'kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null || true' EXIT

ready=""
for _ in $(seq 1 100); do
  if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    echo "server exited before becoming ready -- see $OUT/serve_demo.log" >&2
    cat "$OUT/serve_demo.log" >&2
    exit 1
  fi
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/"; then ready=1; break; fi
  sleep 0.2
done
if [ -z "$ready" ]; then
  echo "server not ready after ~20s -- on a cold target dir the backgrounded" >&2
  echo "cargo release build takes minutes; rerun once 'cargo build --release -p cubism-cli' has finished." >&2
  cat "$OUT/serve_demo.log" >&2
  exit 1
fi

URL="http://127.0.0.1:$PORT/"
echo
echo "============================================================"
echo "  dashboard: $URL"
echo "  time-series defaults: /G · avg_revenue · 2026-04-06..08"
echo "  (day 2026-04-07 serves its corrected revision 2)"
echo "  Ctrl-C to stop"
echo "============================================================"
echo

if [ "$OPEN_BROWSER" = 1 ]; then
  if command -v open >/dev/null 2>&1; then open "$URL"
  elif command -v xdg-open >/dev/null 2>&1; then xdg-open "$URL"
  else echo "(no open/xdg-open found; visit the URL above manually)" >&2
  fi
fi

wait "$SERVE_PID"
