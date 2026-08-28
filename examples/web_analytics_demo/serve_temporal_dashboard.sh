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
# exact JSON bodies the page constructs — including the corrected window at
# its revision 2 — so the chart has real data waiting. It can NOT prove the
# browser actually renders the SVG line chart; that part is verified
# separately (Milestone 16 drove a real browser at it; Milestone 12a's
# precedent for demo/UI slices is a captured transcript, not a cargo test).
#
# The three days sampled are the corrected day and its two neighbours,
# derived from `demo_env.sh` rather than hardcoded — the whole point of the
# capture is the revision-1/2/1 pattern across that boundary, which a fixed
# date silently loses the moment the dataset's shape changes.
#
# Reuses `build_temporal_demo.sh` (Milestone 12a) for the published days —
# this script does not require anything to have run first, but it
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

# START_DATE / DAYS / USERS / LATE_OFFSET / day_at() / LATE_DAY
. ./demo_env.sh
# Sample the corrected day and its neighbours, so the captured revisions
# are 1 / 2 / 1 whatever the range is.
SAMPLE_DAYS="$(day_at "$((LATE_OFFSET - 1))") $LATE_DAY $(day_at "$((LATE_OFFSET + 1))")"

echo "== building the demo days (build_temporal_demo.sh) =="
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
ready=""
for _ in $(seq 1 50); do
  if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    echo "server exited before becoming ready -- see $OUT/serve_dashboard.log" >&2
    cat "$OUT/serve_dashboard.log" >&2
    exit 1
  fi
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/"; then ready=1; break; fi
  sleep 0.2
done
if [ -z "$ready" ]; then
  echo "server not ready after ~10s -- on a cold target dir the backgrounded" >&2
  echo "cargo release build takes minutes; rerun once 'cargo build --release -p cubism-cli' has finished." >&2
  cat "$OUT/serve_dashboard.log" >&2
  exit 1
fi

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
for day in $SAMPLE_DAYS; do
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
python3 - "$SAMPLE_DAYS" "$OUT"/dashboard_day*.json <<'PY'
import json
import sys
from datetime import datetime, timezone

# The corrected day and its two neighbours, passed in from the shell so
# this stays correct whatever `demo_env.sh` selects.
sample_days = sys.argv[1].split()
expected_buckets = [
    int(datetime.strptime(d, "%Y-%m-%d").replace(tzinfo=timezone.utc).timestamp() * 1_000_000)
    for d in sample_days
]
expected_revisions = [1, 2, 1]
files = sys.argv[2:]
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
print(f"all three days answer exactly, with {sample_days[1]} at its corrected revision 2")
PY
