#!/usr/bin/env bash
# Serves the time-series web-analytics demo through `cubism serve` and
# captures what a browser's dashboard would see (`docs/TIMESERIES_ROADMAP.md`
# Milestone 15): the `/` page carrying the time-series panel markup, plus
# the exact per-day `/api/series` requests the panel's JavaScript issues —
# one single-day request per day, because a multi-day range merges into one
# point (`range_query.rs`'s "at most one interior segment").
#
# What this mechanically proves vs what it cannot: the served HTML contains
# the panel hooks (`id="tsPanel"`, `/api/series`) and the API answers the
# exact JSON bodies the page constructs — including window 2026-04-07 at
# its corrected revision 2 — so the chart has real data waiting. It can NOT
# prove the browser actually renders the SVG line chart; that part is
# human-verified (Milestone 12a precedent: demo/UI slices are verified by
# captured transcripts, not cargo tests).
#
# Reuses `build_temporal_demo.sh` (Milestone 12a) for all three published
# days — this script does not require anything to have run first, but it
# DOES clobber the shared `.temporal_build/` directory (`rm -rf` inside),
# same "last one run wins" convention as that script and
# `query_temporal_demo.sh`. The placeholder cube-path parquet is the same
# trick `query_temporal_demo.sh` uses: `serve` requires a real static-cube
# parquet positional even though `/api/series` never reads it.
set -euo pipefail

cd "$(dirname "$0")"
OUT=.temporal_build
BIN="cargo run --release -q -p cubism-cli --"
SPEC=web_analytics_temporal.yaml
PORT=8096

echo "== building all three demo days (build_temporal_demo.sh) =="
SERVE_PID=""
BUILD_LOG=$(mktemp)
trap 'rm -f "$BUILD_LOG"; if [ -n "$SERVE_PID" ]; then kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null || true; fi' EXIT
./build_temporal_demo.sh >"$BUILD_LOG" 2>&1 \
  || { cat "$BUILD_LOG" >&2; exit 1; }
rm -f "$BUILD_LOG"

echo "== writing placeholder cube_path parquet =="
# `python3` here may be an interpreter without pyarrow (e.g. a pyenv shim
# shadowing a miniforge install) — probe candidates and use the first one
# that can actually import it.
PY=
for cand in python3 /opt/homebrew/Caskroom/miniforge/base/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do
  if "$cand" -c "import pyarrow" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "no python3 with pyarrow found (needed for the placeholder parquet)" >&2; exit 1; }
"$PY" - "$OUT/placeholder_cube.parquet" <<'PY'
import sys
import pyarrow as pa
import pyarrow.parquet as pq

pq.write_table(pa.table({"xunit": ["/G"]}), sys.argv[1])
PY

echo "== starting cubism serve on :$PORT =="
$BIN serve "$OUT/placeholder_cube.parquet" --port "$PORT" \
  --spec "$SPEC" --warehouse "$OUT/warehouse" \
  --catalog-db "$OUT/catalog.sqlite" --control-db "$OUT/control.sqlite" \
  >"$OUT/serve_dashboard.log" 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 50); do
  if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    echo "server exited before becoming ready -- see $OUT/serve_dashboard.log" >&2
    cat "$OUT/serve_dashboard.log" >&2
    exit 1
  fi
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/"; then break; fi
  sleep 0.2
done

echo
echo "== dashboard page carries the time-series panel =="
PAGE=$(curl -s "http://127.0.0.1:$PORT/")
for token in 'id="tsPanel"' '/api/series' 'bucket_start' 'is_exact'; do
  if echo "$PAGE" | grep -qF "$token"; then
    echo "  ok: $token"
  else
    echo "  MISSING: $token" >&2
    exit 1
  fi
done

echo
echo "== per-day /api/series requests, exactly as the panel issues them =="
day_request() {
  local day="$1" next="$2"
  cat <<JSON
{"selector":"/G","measure":"avg_revenue","start":$(date_micros "$day"),"end":$(date_micros "$next"),"exact":false,"gap_policy":"missing","windows":[{"window_id":"$day","bucket_start":$(date_micros "$day")}]}
JSON
}
date_micros() {
  python3 -c "import sys; from datetime import datetime, timezone; print(int(datetime.strptime(sys.argv[1], '%Y-%m-%d').replace(tzinfo=timezone.utc).timestamp() * 1_000_000))" "$1"
}

i=0
for day in 2026-04-06 2026-04-07 2026-04-08; do
  next=$(python3 -c "import sys; from datetime import datetime, date, timedelta; d=datetime.strptime(sys.argv[1], '%Y-%m-%d').date(); print(d + timedelta(days=1))" "$day")
  REQ=$(day_request "$day" "$next")
  i=$((i + 1))
  echo "-- day $i ($day) --"
  curl -s -X POST "http://127.0.0.1:$PORT/api/series" -H "Content-Type: application/json" -d "$REQ" \
    | tee "$OUT/dashboard_day${i}.json"
  echo
done

kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null || true
trap - EXIT

echo
python3 - "$OUT"/dashboard_day*.json <<'PY'
import json
import sys

expected_buckets = [1775433600000000, 1775520000000000, 1775606400000000]
expected_revisions = [1, 2, 1]
files = sys.argv[1:]
assert len(files) == 3, f"expected 3 captured responses, got {len(files)}"
print("== summary ==")
ok = True
for i, path in enumerate(files):
    body = json.load(open(path))
    points = body["points"]
    assert len(points) == 1, f"{path}: expected 1 point for a single-day range, got {len(points)}"
    p = points[0]
    bucket_ok = p["bucket_start"] == expected_buckets[i]
    rev = p["published"][0]["revision"] if p["published"] else None
    rev_ok = rev == expected_revisions[i]
    ok &= bucket_ok and rev_ok and p["is_exact"] is True
    print(f"day {i + 1}: value={p['value']!r} is_exact={p['is_exact']} "
          f"bucket_ok={bucket_ok} revision={rev} (expect {expected_revisions[i]})")
assert ok, "dashboard capture failed its assertions"
print("all three days answer exactly, with 2026-04-07 at its corrected revision 2")
PY
